//! Index synchronization: upsert, filters, delete, reindex.

use mm_search::{IndexReport, SearchDocument, SearchIndex, SearchQuery};

fn doc(subject: &str, label: &str, body: &str, class: &str, graph: &str) -> SearchDocument {
    SearchDocument {
        subject: subject.to_owned(),
        graph: graph.to_owned(),
        types: vec![class.to_owned()],
        label: label.to_owned(),
        body: body.to_owned(),
        namespace: "mmd".to_owned(),
        revision: 1,
        updated_at: 0,
        status: "active".to_owned(),
    }
}

#[tokio::test]
async fn upsert_finds_by_label_and_body_and_filters() {
    let temp = tempfile::tempdir().unwrap();
    let index = SearchIndex::open(temp.path()).unwrap();
    index
        .upsert(&doc(
            "https://metamind.dev/data/auth",
            "Auth service",
            "Migrates login sessions to the new provider",
            "https://metamind.dev/ontology#Project",
            "https://metamind.dev/graph/world",
        ))
        .await
        .unwrap();
    index
        .upsert(&doc(
            "https://metamind.dev/data/billing",
            "Billing",
            "Invoice reconciliation and payouts",
            "https://metamind.dev/ontology#Project",
            "https://metamind.dev/graph/world",
        ))
        .await
        .unwrap();

    let hit = index
        .search(&SearchQuery {
            text: "login".into(),
            ..SearchQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(hit.hits.len(), 1);
    assert_eq!(hit.hits[0].subject, "https://metamind.dev/data/auth");
    assert!(hit.hits[0].snippet.as_deref().unwrap().contains("login"));

    let filtered = index
        .search(&SearchQuery {
            text: "service".into(),
            class: Some("https://metamind.dev/ontology#Project".into()),
            graph: Some("https://metamind.dev/graph/world".into()),
            ..SearchQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(filtered.hits.len(), 1);

    let wrong_graph = index
        .search(&SearchQuery {
            text: "service".into(),
            graph: Some("https://metamind.dev/graph/being".into()),
            ..SearchQuery::default()
        })
        .await
        .unwrap();
    assert!(wrong_graph.hits.is_empty());
}

#[tokio::test]
async fn upsert_replaces_rather_than_duplicates_a_subject() {
    let temp = tempfile::tempdir().unwrap();
    let index = SearchIndex::open(temp.path()).unwrap();
    let mut first = doc(
        "https://metamind.dev/data/auth",
        "Auth service",
        "old words",
        "https://metamind.dev/ontology#Project",
        "https://metamind.dev/graph/world",
    );
    index.upsert(&first).await.unwrap();
    first.body = "refresh tokens".into();
    first.revision = 2;
    index.upsert(&first).await.unwrap();
    assert_eq!(index.num_docs().await.unwrap(), 1);

    let old = index
        .search(&SearchQuery {
            text: "old".into(),
            ..SearchQuery::default()
        })
        .await
        .unwrap();
    assert!(old.hits.is_empty());
    let new = index
        .search(&SearchQuery {
            text: "refresh".into(),
            ..SearchQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(new.hits.len(), 1);
    assert_eq!(new.hits[0].revision, 2);
}

#[tokio::test]
async fn delete_and_reindex_reconcile_to_rdf_state() {
    let temp = tempfile::tempdir().unwrap();
    let index = SearchIndex::open(temp.path()).unwrap();
    index
        .upsert(&doc(
            "https://metamind.dev/data/a",
            "Alpha",
            "one",
            "https://metamind.dev/ontology#Note",
            "https://metamind.dev/graph/world",
        ))
        .await
        .unwrap();
    index
        .upsert(&doc(
            "https://metamind.dev/data/b",
            "Beta",
            "two",
            "https://metamind.dev/ontology#Note",
            "https://metamind.dev/graph/world",
        ))
        .await
        .unwrap();

    index.delete("https://metamind.dev/data/a").await.unwrap();
    assert_eq!(index.num_docs().await.unwrap(), 1);

    let report: IndexReport = index
        .reindex(vec![doc(
            "https://metamind.dev/data/a",
            "Alpha again",
            "restored",
            "https://metamind.dev/ontology#Note",
            "https://metamind.dev/graph/world",
        )])
        .await
        .unwrap();
    assert_eq!(report.documents, 1);
    assert_eq!(index.num_docs().await.unwrap(), 1);
    let hit = index
        .search(&SearchQuery {
            text: "restored".into(),
            ..SearchQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(hit.hits[0].subject, "https://metamind.dev/data/a");
}

#[test]
fn query_tokens_are_plain_words() {
    let tokens = mm_search::sanitize_tokens("foo AND (bar* OR \"baz\") -- qux");
    assert_eq!(tokens, vec!["foo", "and", "bar", "or", "baz", "qux"]);
}
