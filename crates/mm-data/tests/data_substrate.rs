//! Phase 3: typed entities, graph routing, the outbox, snapshots, and the catalog.

use mm_data::{
    initialize_data_layer, DataBroker, DataCreateRequest, DataDeleteMode, DataDeleteRequest,
    DataMutationResult, DataPatch, DataQuery, DataSearchRequest, DataService, DataUpdateMode,
    DataUpdateRequest, DataValue, IndexHealth, InitConfig, NamespaceBinding, SchemaInclude,
    SchemaSpec, ValidationTarget,
};

fn init_config(temp: &tempfile::TempDir) -> InitConfig {
    InitConfig {
        base_dir: Some(temp.path().to_path_buf()),
        in_memory: true,
        ..InitConfig::default()
    }
}

fn create(class: &str, label: &str) -> DataCreateRequest {
    DataCreateRequest {
        r#type: class.to_string(),
        id: None,
        properties: vec![DataPatch {
            predicate: "rdfs:label".to_string(),
            values: vec![DataValue::text(label)],
        }],
        schema: None,
        graph: None,
        idempotency_key: None,
    }
}

async fn create_entity(service: &DataService, class: &str, label: &str) -> DataMutationResult {
    service.create(create(class, label)).await.expect("create")
}

#[tokio::test]
async fn typed_entities_route_to_their_graphs_and_round_trip() {
    let temp = tempfile::tempdir().unwrap();
    let data = initialize_data_layer(init_config(&temp)).await.unwrap();
    let service = &data.data_service;

    let note = create_entity(service, "mm:Note", "Deploy notes").await;
    assert_eq!(
        note.entity.graph, "https://metamind.dev/graph/world",
        "an ordinary class routes to world"
    );
    assert_eq!(
        note.entity.types,
        vec!["https://metamind.dev/ontology#Note"]
    );
    assert!(note.entity.iri.starts_with("https://metamind.dev/data/"));

    let mut user_request = create("mm:User", "Ada");
    user_request.properties.push(DataPatch {
        predicate: "foaf:name".into(),
        values: vec![DataValue::text("Ada")],
    });
    let user = service.create(user_request).await.unwrap();
    assert_eq!(user.entity.graph, "https://metamind.dev/graph/users/index");
    assert!(user
        .entity
        .iri
        .starts_with("https://metamind.dev/data/user/"));

    // Management classes are owned by their services, but their routing is
    // still part of the broker's table.
    for (class, graph) in [
        ("mm:Plan", "plan"),
        ("mm:ScheduledAction", "action"),
        ("mm:Session", "session"),
        ("mm:LogEntry", "log"),
        ("mm:Schema", "schema"),
    ] {
        assert_eq!(
            mm_data::data::rdf_mapper::graph_for_class(class),
            graph,
            "{class}"
        );
    }
    let claim = create_entity(service, "mm:Claim", "Staging resets").await;
    assert_eq!(claim.entity.graph, "https://metamind.dev/graph/epistemic");
    let memory = create_entity(service, "mm:Memory", "Remembered").await;
    assert_eq!(memory.entity.graph, "https://metamind.dev/graph/memory");

    // A typed property survives RDF and back.
    let mut request = create("mm:Note", "Typed");
    request.properties.push(DataPatch {
        predicate: "mm:confidence".to_string(),
        values: vec![DataValue::Literal {
            value: "0.75".to_string(),
            datatype: Some("http://www.w3.org/2001/XMLSchema#double".to_string()),
            language: None,
        }],
    });
    let typed = service.create(request).await.unwrap();
    let loaded = service
        .find_entity(&typed.entity.iri)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.types, vec!["https://metamind.dev/ontology#Note"]);
    assert_eq!(loaded.label.as_deref(), Some("Typed"));
    assert!(
        loaded
            .properties
            .contains_key("https://metamind.dev/ontology#confidence"),
        "{:?}",
        loaded.properties
    );
}

#[tokio::test]
async fn create_update_delete_track_the_outbox_and_search() {
    let temp = tempfile::tempdir().unwrap();
    let data = initialize_data_layer(init_config(&temp)).await.unwrap();
    let service = &data.data_service;
    assert!(service.index_ready(), "the index should be ready");

    let created = create_entity(service, "mm:Project", "Auth service").await;
    assert_eq!(created.index, IndexHealth::Healthy);
    // A row per mutation exists (done after the synchronous index attempt).
    assert!(
        service.outbox().max_seq().await.unwrap() >= created.entity.revision,
        "the outbox saw the dataRevision"
    );
    assert_eq!(
        service.outbox().pending_count().await.unwrap(),
        0,
        "the synchronous index attempt marked it done"
    );

    let found = service
        .search(DataSearchRequest {
            query: "auth".into(),
            class: None,
            graph: None,
            namespace: None,
            limit: 10,
            snippet: true,
            include_deleted: false,
        })
        .await
        .unwrap();
    assert_eq!(found.hits.len(), 1);
    assert_eq!(found.hits[0].iri, created.entity.iri);

    let updated = service
        .update(DataUpdateRequest {
            subject: created.entity.iri.clone(),
            properties: vec![DataPatch {
                predicate: "rdfs:label".into(),
                values: vec![DataValue::text("Billing service")],
            }],
            mode: DataUpdateMode::Merge,
            expected_revision: Some(created.entity.revision),
            idempotency_key: None,
        })
        .await
        .unwrap();
    assert!(updated.entity.revision > created.entity.revision);

    let old = service
        .search(DataSearchRequest {
            query: "auth".into(),
            class: None,
            graph: None,
            namespace: None,
            limit: 10,
            snippet: true,
            include_deleted: false,
        })
        .await
        .unwrap();
    assert!(old.hits.is_empty(), "old text disappears from the index");
    let new = service
        .search(DataSearchRequest {
            query: "billing".into(),
            class: None,
            graph: None,
            namespace: None,
            limit: 10,
            snippet: true,
            include_deleted: false,
        })
        .await
        .unwrap();
    assert_eq!(new.hits.len(), 1);

    // A stale revision is a conflict, not a silent overwrite.
    let conflict = service
        .update(DataUpdateRequest {
            subject: created.entity.iri.clone(),
            properties: vec![DataPatch {
                predicate: "rdfs:label".into(),
                values: vec![DataValue::text("Stale")],
            }],
            mode: DataUpdateMode::Merge,
            expected_revision: Some(created.entity.revision),
            idempotency_key: None,
        })
        .await;
    assert!(matches!(
        conflict,
        Err(mm_data::data::DataError::Conflict(_))
    ));

    // Soft delete is readable only with include_deleted.
    service
        .delete(DataDeleteRequest {
            subject: created.entity.iri.clone(),
            mode: DataDeleteMode::Soft,
            expected_revision: None,
            reason: Some("test".into()),
            idempotency_key: None,
            authority: None,
        })
        .await
        .unwrap();
    let query = DataQuery {
        include_deleted: false,
        ..DataQuery::default()
    };
    assert!(service.query(query).await.unwrap().entities.is_empty());
    let deleted = service
        .query(DataQuery {
            include_deleted: true,
            ..DataQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(deleted.entities.len(), 1);
    assert_eq!(deleted.entities[0].status, mm_data::EntityStatus::Deleted);

    // Hard delete needs authority.
    let note = create_entity(service, "mm:Note", "Temporary").await;
    let refused = service
        .delete(DataDeleteRequest {
            subject: note.entity.iri.clone(),
            mode: DataDeleteMode::Hard,
            expected_revision: None,
            reason: None,
            idempotency_key: None,
            authority: None,
        })
        .await;
    assert!(refused.is_err());
    service
        .delete(DataDeleteRequest {
            subject: note.entity.iri.clone(),
            mode: DataDeleteMode::Hard,
            expected_revision: None,
            reason: None,
            idempotency_key: None,
            authority: Some("user".into()),
        })
        .await
        .unwrap();
    assert!(service
        .find_entity(&note.entity.iri)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_dead_index_worker_leaves_rdf_and_the_outbox_and_retries_on_boot() {
    let temp = tempfile::tempdir().unwrap();
    // Pre-create the index path as a file: opening the index fails, which is
    // exactly what a dead index worker looks like to the write path.
    let search_dir = temp.path().join("search");
    std::fs::create_dir_all(&search_dir).unwrap();
    std::fs::write(search_dir.join("tantivy"), b"not a directory").unwrap();

    let data = initialize_data_layer(init_config(&temp)).await.unwrap();
    let service = &data.data_service;
    assert!(service.search_index().is_none());
    let created = create_entity(service, "mm:Project", "Auth service").await;
    assert_eq!(created.index, IndexHealth::Unavailable);
    assert_eq!(
        service.outbox().pending_count().await.unwrap(),
        1,
        "the durable row is still pending"
    );
    assert!(
        service
            .find_entity(&created.entity.iri)
            .await
            .unwrap()
            .is_some(),
        "RDF committed even though the index did not"
    );

    // Repair the path and reopen over the same engine and SQLite store.
    std::fs::remove_file(search_dir.join("tantivy")).unwrap();
    let reopened = DataService::open(
        std::sync::Arc::clone(&data.engine),
        &data.sqlite,
        &data.base_dir,
    )
    .await
    .unwrap();
    assert!(reopened.search_index().is_some());
    assert_eq!(reopened.outbox().pending_count().await.unwrap(), 0);
    let hits = reopened
        .search(DataSearchRequest {
            query: "auth".into(),
            class: None,
            graph: None,
            namespace: None,
            limit: 10,
            snippet: false,
            include_deleted: false,
        })
        .await
        .unwrap();
    assert_eq!(hits.hits.len(), 1);
}

#[tokio::test]
async fn the_rdfxml_snapshot_imports_into_an_empty_store_without_duplicates() {
    let temp = tempfile::tempdir().unwrap();
    let data = initialize_data_layer(init_config(&temp)).await.unwrap();
    let created = create_entity(&data.data_service, "mm:Project", "Aurora").await;
    data.data_service.checkpoint().await.unwrap();
    assert!(temp.path().join("cubecast/metamind.rdf").exists());
    assert!(temp.path().join("cubecast/metamind.nq").exists());

    // A second boot with a fresh in-memory RocksDB store imports the snapshot.
    let reopened = initialize_data_layer(init_config(&temp)).await.unwrap();
    let entities = reopened
        .data_service
        .query(DataQuery {
            include_deleted: true,
            ..DataQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(entities.entities.len(), 1, "{:?}", entities.entities);
    assert_eq!(entities.entities[0].iri, created.entity.iri);
    assert_eq!(entities.entities[0].graph, created.entity.graph);
}

#[tokio::test]
async fn the_schema_catalog_and_namespace_registry_are_rdf_backed() {
    let temp = tempfile::tempdir().unwrap();
    let data = initialize_data_layer(init_config(&temp)).await.unwrap();
    let service = &data.data_service;

    let schemas = service.list_schemas().await.unwrap();
    assert!(schemas.iter().any(|schema| schema.prefix == "mm"));
    assert!(schemas.iter().any(|schema| schema.prefix == "foaf"));
    assert!(schemas.iter().all(|schema| schema.active));

    let spec = SchemaSpec {
        id: "ex".into(),
        prefix: "ex".into(),
        namespace: "https://example.com/ns#".into(),
        version: "1".into(),
        source: "test".into(),
        body: r#"
            @prefix owl: <http://www.w3.org/2002/07/owl#> .
            @prefix ex:  <https://example.com/ns#> .
            ex:Widget a owl:Class .
            ex:size a owl:DatatypeProperty .
            <https://example.com/ns> owl:imports <https://example.com/base> .
        "#
        .into(),
        format: Some("turtle".into()),
    };
    let registered = service.catalog().register(spec.clone()).await.unwrap();
    assert!(!registered.active);
    assert_eq!(registered.class_count, 1);
    assert_eq!(registered.property_count, 1);
    let detail = service.get_schema("ex", SchemaInclude::All).await.unwrap();
    assert!(detail
        .classes
        .contains(&"https://example.com/ns#Widget".into()));
    assert_eq!(detail.imports.len(), 1);

    let activated = service.catalog().activate("ex").await.unwrap();
    assert!(activated.active);

    // A registered, active namespace resolves for routing and validation.
    let widget = create_entity(service, "ex:Widget", "Gear").await;
    assert_eq!(widget.entity.types, vec!["https://example.com/ns#Widget"]);
    assert_eq!(widget.entity.graph, "https://metamind.dev/graph/world");

    // Namespace collisions and immutability.
    service
        .namespaces()
        .register(NamespaceBinding {
            prefix: "exp".into(),
            namespace: "https://example.com/project#".into(),
            builtin: false,
        })
        .await
        .unwrap();
    assert!(service
        .namespaces()
        .register(NamespaceBinding {
            prefix: "exp".into(),
            namespace: "https://example.com/other#".into(),
            builtin: false,
        })
        .await
        .is_err());
    assert!(service
        .namespaces()
        .register(NamespaceBinding {
            prefix: "mm".into(),
            namespace: "https://example.com/mm#".into(),
            builtin: true,
        })
        .await
        .is_err());

    // Unknown classes are refused; a declared built-in class expands.
    assert!(service.create(create("nope:Thing", "x")).await.is_err());
    assert_eq!(
        service.expand("mm:Note").await.unwrap(),
        "https://metamind.dev/ontology#Note"
    );
    assert!(service
        .validate(ValidationTarget::Graph("world".into()))
        .await
        .is_ok());
}

#[tokio::test]
async fn identity_minting_is_canonical_and_legacy_aliases_resolve() {
    let temp = tempfile::tempdir().unwrap();
    let data = initialize_data_layer(init_config(&temp)).await.unwrap();
    let namespaces = data.data_service.namespaces();

    let minted = namespaces.mint("user", None).await.unwrap();
    assert!(minted.starts_with("https://metamind.dev/data/user/"));
    assert_eq!(
        namespaces.mint("user", Some("abc")).await.unwrap(),
        "https://metamind.dev/data/user/abc"
    );
    assert_eq!(
        namespaces
            .mint("project", Some("urn:metamind:project:legacy"))
            .await
            .unwrap(),
        "https://metamind.dev/data/project/legacy"
    );
    assert_eq!(
        namespaces.normalize("urn:metamind:user:old").await.unwrap(),
        "https://metamind.dev/data/user/old"
    );
    assert!(namespaces.mint("user", Some("not allowed")).await.is_err());
}

#[tokio::test]
async fn a_second_being_is_refused_as_a_singleton() {
    let temp = tempfile::tempdir().unwrap();
    let data = initialize_data_layer(init_config(&temp)).await.unwrap();
    let service = &data.data_service;
    let first = create_entity(service, "mm:Being", "Metamind").await;
    let second = service.create(create("mm:Being", "Another")).await;
    assert!(
        matches!(second, Err(mm_data::data::DataError::SchemaViolation(_))),
        "{second:?}"
    );
    assert_eq!(
        service
            .find_entity(&first.entity.iri)
            .await
            .unwrap()
            .unwrap()
            .iri,
        first.entity.iri
    );
}

#[tokio::test]
async fn idempotency_keys_replay_the_same_entity() {
    let temp = tempfile::tempdir().unwrap();
    let data = initialize_data_layer(init_config(&temp)).await.unwrap();
    let service = &data.data_service;
    let mut request = create("mm:Note", "Idempotent");
    request.idempotency_key = Some("key-1".into());
    let first = service.create(request.clone()).await.unwrap();
    let second = service.create(request).await.unwrap();
    assert!(second.unchanged);
    assert_eq!(second.entity.iri, first.entity.iri);
}
