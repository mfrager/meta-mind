//! Queue ordering, leases, dedupe and batch selection.
//!
//! Queue state lives on the instance document, so a restart reads the same
//! ordering it wrote. A lease is a timestamp, not a process-local flag: a crashed
//! worker's lease expires and the next claim reclaims the instance. Higher
//! priorities run first; every aging window in the queue adds one, so a low
//! priority instance cannot starve forever.
//!
//! Dedupe is checked against live instances only. Two requests for the same
//! template/session/key return the existing process instead of starting a second
//! one.

use std::collections::BTreeMap;

use mm_data::process::{ProcessError, ProcessInstance, ProcessStatus, QueueState};

/// The effective priority at `now`.
pub fn effective_priority(queue: &QueueState, now: i64, aging_ms: i64) -> i64 {
    let waited = now.saturating_sub(queue.enqueued_at).max(0);
    let periods = if aging_ms > 0 { waited / aging_ms } else { 0 };
    queue.priority.saturating_add(periods)
}

/// Whether a queued instance may be claimed.
pub fn claimable(instance: &ProcessInstance, now: i64) -> bool {
    instance.status == ProcessStatus::Queued && !instance.leased(now)
}

/// Claimable instances, best first.
pub fn order<'a>(
    instances: impl IntoIterator<Item = &'a ProcessInstance>,
    now: i64,
    aging_ms: i64,
) -> Vec<&'a ProcessInstance> {
    let mut candidates: Vec<&ProcessInstance> = instances
        .into_iter()
        .filter(|instance| claimable(instance, now))
        .collect();
    candidates.sort_by(|a, b| {
        effective_priority(&b.queue, now, aging_ms)
            .cmp(&effective_priority(&a.queue, now, aging_ms))
            .then(a.queue.enqueued_at.cmp(&b.queue.enqueued_at))
            .then(a.id.cmp(&b.id))
    });
    candidates
}

/// The live instance with this dedupe key, if any.
pub fn dedupe_conflict<'a>(
    instances: impl IntoIterator<Item = &'a ProcessInstance>,
    dedupe_key: &str,
) -> Option<&'a ProcessInstance> {
    instances.into_iter().find(|instance| {
        instance.status.is_live() && instance.queue.dedupe_key.as_deref() == Some(dedupe_key)
    })
}

/// A claimed batch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Batch {
    /// Instance ids to start, in claim order.
    pub ids: Vec<String>,
    /// Slots left after the batch.
    pub remaining: usize,
}

/// Claim up to `slots` instances.
///
/// Steps with the same `batch_key` are kept together up to `batch_size`, so one
/// action batch is not interleaved with another. Leases are written onto the
/// instances in place; the caller persists them.
pub fn claim_batch(
    instances: &mut [ProcessInstance],
    now: i64,
    owner: &str,
    slots: usize,
    batch_size: usize,
    aging_ms: i64,
    lease_ms: i64,
) -> Result<Batch, ProcessError> {
    if slots == 0 {
        return Ok(Batch::default());
    }
    let mut chosen: Vec<usize> = Vec::new();
    let mut seen_groups: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_session: BTreeMap<&str, usize> = BTreeMap::new();
    let mut order: Vec<usize> = (0..instances.len()).collect();
    order.sort_by(|left, right| {
        let a = &instances[*left];
        let b = &instances[*right];
        effective_priority(&b.queue, now, aging_ms)
            .cmp(&effective_priority(&a.queue, now, aging_ms))
            .then(a.queue.enqueued_at.cmp(&b.queue.enqueued_at))
            .then(a.id.cmp(&b.id))
    });
    for index in order {
        if chosen.len() >= slots {
            break;
        }
        let instance = &instances[index];
        if !claimable(instance, now) {
            continue;
        }
        // One live claim per session unless the queue has room for the whole
        // batch; this keeps a single session from taking every worker.
        let used = per_session.entry(instance.session_id.as_str()).or_default();
        if *used >= batch_size {
            continue;
        }
        if let Some(batch) = instance.queue.batch_key.as_deref() {
            let used = seen_groups.entry(batch.to_owned()).or_default();
            if *used >= batch_size {
                continue;
            }
            *used += 1;
        }
        *used += 1;
        chosen.push(index);
    }
    for index in &chosen {
        let instance = &mut instances[*index];
        instance.queue.lease_owner = Some(owner.to_owned());
        instance.queue.leased_until = Some(now.saturating_add(lease_ms));
        instance.queue.claims = instance.queue.claims.saturating_add(1);
    }
    let ids = chosen
        .iter()
        .map(|index| instances[*index].id.clone())
        .collect();
    Ok(Batch {
        ids,
        remaining: slots.saturating_sub(chosen.len()),
    })
}

/// Release an expired lease so the instance can be reclaimed.
pub fn reclaim_expired(instance: &mut ProcessInstance, now: i64) -> bool {
    if instance.status == ProcessStatus::Queued
        && instance
            .queue
            .leased_until
            .is_some_and(|until| until <= now)
    {
        instance.queue.lease_owner = None;
        instance.queue.leased_until = None;
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_data::process::{ProcessTemplate, TemplateStep};

    fn instance(id: &str, session: &str, priority: i64, at: i64) -> ProcessInstance {
        let template = ProcessTemplate::new(
            "urn:metamind:process/template/t",
            "T",
            vec![TemplateStep::new("a", "A", "do a")],
            0,
        );
        let mut instance = ProcessInstance::new(id, &template, session, at);
        instance.queue.priority = priority;
        instance
    }

    #[test]
    fn priority_then_age_orders_the_queue() {
        let mut low = instance("low", "s", 0, 0);
        low.queue.enqueued_at = 0;
        let high = instance("high", "s", 5, 100);
        let ordered = order([&low, &high], 100, 10_000);
        assert_eq!(ordered[0].id, "high");
        let fresh = instance("fresh", "s", 5, 60_000);
        let ordered = order([&low, &fresh], 60_000, 10_000);
        assert_eq!(ordered[0].id, "low");
    }

    #[test]
    fn leases_expire_and_can_be_reclaimed() {
        let mut item = instance("p", "s", 0, 0);
        item.queue.leased_until = Some(10);
        assert!(!claimable(&item, 9));
        assert!(reclaim_expired(&mut item, 10));
        assert!(claimable(&item, 10));
    }

    #[test]
    fn dedupe_only_matches_live_instances() {
        let mut first = instance("one", "s", 0, 0);
        first.queue.dedupe_key = Some("k".into());
        let mut second = instance("two", "s", 0, 0);
        second.queue.dedupe_key = Some("k".into());
        assert_eq!(
            dedupe_conflict([&first, &second], "k").map(|item| item.id.as_str()),
            Some("one")
        );
        second.status = ProcessStatus::Completed;
        assert_eq!(
            dedupe_conflict([&first, &second], "k").map(|item| item.id.as_str()),
            Some("one")
        );
        first.status = ProcessStatus::Completed;
        assert!(dedupe_conflict([&first, &second], "k").is_none());
    }

    #[test]
    fn batch_claims_at_most_the_slots() {
        let mut items: Vec<ProcessInstance> = (0..10)
            .map(|index| instance(&format!("p{index}"), "s", 0, index))
            .collect();
        let batch = claim_batch(&mut items, 100, "worker", 4, 4, 1_000, 1_000).unwrap();
        assert_eq!(batch.ids.len(), 4);
        assert_eq!(batch.remaining, 0);
        assert_eq!(
            items
                .iter()
                .filter(|item| item.queue.lease_owner.as_deref() == Some("worker"))
                .count(),
            4
        );
    }
}
