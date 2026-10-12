import { useCallback, useEffect, useReducer, useState } from "react";

import { adapterFor } from "@/components/trees/adapters/index.ts";
import { applyTreeOps } from "@/lib/api/trees.ts";
import {
  initialTreeState,
  postFor,
  reduceTree,
  type TreeAction,
  type TreeSnapshot,
  type TreeState,
} from "@/lib/trees/model.ts";

export function useTreeController(name: string) {
  const [snapshot, setSnapshot] = useState<TreeSnapshot | null>(null);
  const [state, dispatch] = useReducer(reduceTree, initialTreeState);
  const [error, setError] = useState("");
  const [eventId, setEventId] = useState("");

  useEffect(() => {
    let cancelled = false;
    setError("");
    void adapterFor(name)
      .load()
      .then((tree) => {
        if (cancelled) return;
        setSnapshot(tree);
        dispatch({ type: "load", nodes: tree.nodes });
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(err instanceof Error ? err.message : "The tree did not load.");
      });
    return () => {
      cancelled = true;
    };
  }, [name]);

  const commit = useCallback(
    (action: TreeAction) => {
      const ops = postFor(state, action);
      const previous = state;
      dispatch(action);
      if (!ops || ops.length === 0) return;
      void applyTreeOps(name, ops)
        .then((saved) => {
          setSnapshot(saved.tree);
          setEventId(saved.eventId);
          setError("");
        })
        .catch((err: unknown) => {
          dispatch({ type: "revert", state: previous });
          setError(err instanceof Error ? err.message : "The edit was not saved.");
        });
    },
    [name, state],
  );

  return { snapshot, state, error, eventId, dispatch, commit };
}

export type { TreeState };
