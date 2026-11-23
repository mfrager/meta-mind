//! RDF/XML, transactional batches, and the lossless N-Quads backup.

use std::path::Path;

use mm_store_graph::{GraphBatch, GraphStore};
use oxigraph::io::{RdfFormat, RdfParser};
use oxigraph::model::Quad;

const RDFXML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
         xmlns:mm="https://metamind.dev/ontology#"
         xmlns:mmd="https://metamind.dev/data/">
  <rdf:Description rdf:about="https://metamind.dev/data/note1">
    <rdf:type rdf:resource="https://metamind.dev/ontology#Note"/>
    <mm:name>Alpha</mm:name>
  </rdf:Description>
</rdf:RDF>
"#;

async fn open_store() -> GraphStore {
    GraphStore::in_memory(Path::new("unused-shapes.ttl"))
        .await
        .expect("store")
}

#[tokio::test]
async fn turtle_and_rdfxml_load_through_the_same_store() {
    let store = open_store().await;
    let turtle = r#"
        @prefix mm:  <https://metamind.dev/ontology#> .
        @prefix mmd: <https://metamind.dev/data/> .
        mmd:note2 a mm:Note ; mm:name "Beta" .
    "#;
    let turtle_count = store.handle().insert_turtle("world", turtle).await.unwrap();
    let xml_count = store.handle().insert_rdfxml("world", RDFXML).await.unwrap();
    assert_eq!(turtle_count, 2);
    assert_eq!(xml_count, 2);
    assert_eq!(store.graph_quad_count("world").await.unwrap(), 4);
    let rows = store.triples("world").await.unwrap();
    assert!(rows.iter().any(|row| row.to_string().contains("Alpha")));
    assert!(rows.iter().any(|row| row.to_string().contains("Beta")));
    store.shutdown().await.unwrap();
}

#[tokio::test]
async fn rdfxml_round_trips_through_parse_and_dump() {
    let store = open_store().await;
    store.handle().insert_rdfxml("world", RDFXML).await.unwrap();
    let dumped = store.dump_rdfxml("world").await.unwrap();
    assert!(dumped.contains("rdf:RDF"), "{dumped}");
    assert!(dumped.contains("Alpha"), "{dumped}");

    let other = open_store().await;
    other
        .handle()
        .insert_rdfxml("world", &dumped)
        .await
        .unwrap();
    assert_eq!(other.graph_quad_count("world").await.unwrap(), 2);
    store.shutdown().await.unwrap();
    other.shutdown().await.unwrap();
}

#[tokio::test]
async fn nquads_backup_preserves_graph_names() {
    let store = open_store().await;
    store
        .handle()
        .insert_turtle("world", "<https://a> <https://p> <https://o> .")
        .await
        .unwrap();
    store.handle().insert_rdfxml("being", RDFXML).await.unwrap();
    let backup = store.dump_nquads().await.unwrap();
    assert!(backup.contains("graph/world"), "{backup}");
    assert!(backup.contains("graph/being"), "{backup}");

    let restored = open_store().await;
    let mut count = 0;
    for quad in RdfParser::from_format(RdfFormat::NQuads).for_slice(backup.as_bytes()) {
        let quad: Quad = quad.unwrap();
        restored.handle().insert(quad).await.unwrap();
        count += 1;
    }
    assert_eq!(count, 3);
    assert_eq!(restored.graph_quad_count("world").await.unwrap(), 1);
    assert_eq!(restored.graph_quad_count("being").await.unwrap(), 2);
    store.shutdown().await.unwrap();
    restored.shutdown().await.unwrap();
}

#[tokio::test]
async fn apply_batch_deletes_and_inserts_in_one_transaction() {
    let store = open_store().await;
    store
        .handle()
        .insert_turtle(
            "world",
            r#"@prefix mmd: <https://metamind.dev/data/> .
               @prefix mm:  <https://metamind.dev/ontology#> .
               mmd:note1 mm:name "Old" ."#,
        )
        .await
        .unwrap();
    let old = store
        .quads_for_subject("world", "https://metamind.dev/data/note1")
        .await
        .unwrap();
    assert_eq!(old.len(), 1);

    let new = Quad::new(
        oxigraph::model::NamedNode::new("https://metamind.dev/data/note1").unwrap(),
        oxigraph::model::NamedNode::new("https://metamind.dev/ontology#name").unwrap(),
        oxigraph::model::Literal::new_simple_literal("New"),
        oxigraph::model::GraphName::NamedNode(
            oxigraph::model::NamedNode::new("https://metamind.dev/graph/world").unwrap(),
        ),
    );
    let report = store
        .handle()
        .apply_batch(GraphBatch {
            deletes: old,
            inserts: vec![new],
        })
        .await
        .unwrap();
    assert_eq!((report.deleted, report.inserted), (1, 1));
    let rows = store.triples("world").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].to_string().contains("New"), "{rows:?}");
    store.shutdown().await.unwrap();
}
