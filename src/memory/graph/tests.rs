use super::*;

fn entity(name: &str) -> Entity {
    Entity {
        name: name.to_string(),
        entity_type: "person".to_string(),
        observations: Vec::new(),
    }
}

#[tokio::test]
async fn create_entities_deduplicates() {
    let dir = tempfile::tempdir().unwrap();
    let manager = KnowledgeGraphManager::new(dir.path().join("memory.jsonl"));

    let added = manager
        .create_entities(vec![entity("alice"), entity("alice"), entity("bob")])
        .await
        .unwrap();
    assert_eq!(added.len(), 2);

    // Same names again are ignored.
    let added = manager
        .create_entities(vec![entity("alice"), entity("carol")])
        .await
        .unwrap();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].name, "carol");

    let graph = manager.read_graph().await.unwrap();
    assert_eq!(graph.entities.len(), 3);
}

#[tokio::test]
async fn relations_deduplicate() {
    let dir = tempfile::tempdir().unwrap();
    let manager = KnowledgeGraphManager::new(dir.path().join("memory.jsonl"));
    manager
        .create_entities(vec![entity("alice"), entity("bob")])
        .await
        .unwrap();

    let rel = || Relation {
        from: "alice".into(),
        to: "bob".into(),
        relation_type: "works_with".into(),
    };
    let added = manager.create_relations(vec![rel(), rel()]).await.unwrap();
    assert_eq!(added.len(), 1);
    let added = manager.create_relations(vec![rel()]).await.unwrap();
    assert_eq!(added.len(), 0, "exact duplicates are skipped");
}

#[tokio::test]
async fn relations_may_reference_entities_not_yet_present() {
    let dir = tempfile::tempdir().unwrap();
    let manager = KnowledgeGraphManager::new(dir.path().join("memory.jsonl"));
    manager
        .create_entities(vec![entity("alice")])
        .await
        .unwrap();
    let added = manager
        .create_relations(vec![Relation {
            from: "alice".into(),
            to: "missing".into(),
            relation_type: "knows".into(),
        }])
        .await
        .unwrap();
    assert_eq!(added.len(), 1);
}

#[tokio::test]
async fn add_observations_fails_for_missing_entity() {
    let dir = tempfile::tempdir().unwrap();
    let manager = KnowledgeGraphManager::new(dir.path().join("memory.jsonl"));
    manager
        .create_entities(vec![entity("alice")])
        .await
        .unwrap();

    let err = manager
        .add_observations(vec![ObservationInput {
            entity_name: "nobody".into(),
            contents: vec!["x".into()],
        }])
        .await
        .unwrap_err();
    assert!(err.contains("Entity with name nobody not found"));

    let results = manager
        .add_observations(vec![ObservationInput {
            entity_name: "alice".into(),
            contents: vec![
                "speaks Spanish".into(),
                "speaks Spanish".into(),
                "likes tea".into(),
            ],
        }])
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].added_observations.len(), 2);
    assert_eq!(
        results[0].added_observations,
        ["speaks Spanish", "likes tea"]
    );
}

#[tokio::test]
async fn delete_entities_cascades_relations() {
    let dir = tempfile::tempdir().unwrap();
    let manager = KnowledgeGraphManager::new(dir.path().join("memory.jsonl"));
    manager
        .create_entities(vec![entity("alice"), entity("bob")])
        .await
        .unwrap();
    manager
        .create_relations(vec![Relation {
            from: "alice".into(),
            to: "bob".into(),
            relation_type: "knows".into(),
        }])
        .await
        .unwrap();

    manager.delete_entities(vec!["alice".into()]).await.unwrap();
    let graph = manager.read_graph().await.unwrap();
    assert_eq!(graph.entities.len(), 1);
    assert!(graph.relations.is_empty(), "relations cascade-deleted");

    // Silent when entity does not exist.
    manager.delete_entities(vec!["ghost".into()]).await.unwrap();
}

#[tokio::test]
async fn search_and_open_nodes() {
    let dir = tempfile::tempdir().unwrap();
    let manager = KnowledgeGraphManager::new(dir.path().join("memory.jsonl"));
    manager
        .create_entities(vec![
            Entity {
                name: "alice".into(),
                entity_type: "person".into(),
                observations: vec!["speaks Spanish".into()],
            },
            Entity {
                name: "acme".into(),
                entity_type: "organization".into(),
                observations: vec!["sells widgets".into()],
            },
            entity("bob"),
        ])
        .await
        .unwrap();
    manager
        .create_relations(vec![
            Relation {
                from: "alice".into(),
                to: "acme".into(),
                relation_type: "works_at".into(),
            },
            Relation {
                from: "alice".into(),
                to: "bob".into(),
                relation_type: "knows".into(),
            },
        ])
        .await
        .unwrap();

    // Search by name.
    let graph = manager.search_nodes("ALICE").await.unwrap();
    assert_eq!(graph.entities.len(), 1);
    // Relations to nodes outside the result set are included.
    assert_eq!(graph.relations.len(), 2);

    // Search by observation content.
    let graph = manager.search_nodes("spanish").await.unwrap();
    assert_eq!(graph.entities.len(), 1);
    assert_eq!(graph.entities[0].name, "alice");

    // Search with no matches.
    let graph = manager.search_nodes("nothing").await.unwrap();
    assert!(graph.entities.is_empty());
    assert!(graph.relations.is_empty());

    // Open nodes returns requested entities and their relations.
    let graph = manager
        .open_nodes(vec!["acme".into(), "ghost".into()])
        .await
        .unwrap();
    assert_eq!(graph.entities.len(), 1);
    assert_eq!(graph.entities[0].name, "acme");
    assert_eq!(graph.relations.len(), 1);
}

#[tokio::test]
async fn persistence_across_manager_instances() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.jsonl");
    {
        let manager = KnowledgeGraphManager::new(path.clone());
        manager
            .create_entities(vec![entity("alice")])
            .await
            .unwrap();
    }
    {
        let manager = KnowledgeGraphManager::new(path);
        let graph = manager.read_graph().await.unwrap();
        assert_eq!(graph.entities.len(), 1);
        assert_eq!(graph.entities[0].name, "alice");
    }
}

#[tokio::test]
async fn missing_file_loads_empty_graph() {
    let dir = tempfile::tempdir().unwrap();
    let manager = KnowledgeGraphManager::new(dir.path().join("does-not-exist.jsonl"));
    let graph = manager.read_graph().await.unwrap();
    assert!(graph.entities.is_empty());
    assert!(graph.relations.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn memory_rewrites_preserve_private_file_mode() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.jsonl");
    let manager = KnowledgeGraphManager::new(path.clone());
    manager
        .create_entities(vec![entity("alice")])
        .await
        .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

    // A mutation rewrites the persistence file atomically; it must not
    // make an existing private file more permissive.
    manager.create_entities(vec![entity("bob")]).await.unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "mode {mode:#o}");
    let graph = manager.read_graph().await.unwrap();
    assert_eq!(graph.entities.len(), 2, "rewrite content is intact");
}

#[tokio::test]
async fn mutation_rewrites_produce_valid_complete_jsonl_without_temp_leftovers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.jsonl");
    let manager = KnowledgeGraphManager::new(path.clone());
    manager
        .create_entities(vec![entity("alice"), entity("bob")])
        .await
        .unwrap();
    manager
        .create_relations(vec![Relation {
            from: "alice".into(),
            to: "bob".into(),
            relation_type: "knows".into(),
        }])
        .await
        .unwrap();
    manager
        .add_observations(vec![ObservationInput {
            entity_name: "alice".into(),
            contents: vec!["speaks Spanish".into()],
        }])
        .await
        .unwrap();

    // The target is complete, parseable JSONL after every rewrite.
    let raw = tokio::fs::read_to_string(&path).await.unwrap();
    let lines: Vec<&str> = raw.lines().filter(|line| !line.trim().is_empty()).collect();
    assert_eq!(lines.len(), 3, "{raw}");
    for line in &lines {
        serde_json::from_str::<GraphItem>(line).expect("each line parses as a graph item");
    }

    // No temporary files are left behind in the target directory.
    let entries: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, ["memory.jsonl"], "{entries:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn failed_write_preserves_the_existing_valid_target() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.jsonl");
    let manager = KnowledgeGraphManager::new(path.clone());
    manager
        .create_entities(vec![entity("alice")])
        .await
        .unwrap();

    // A read-only directory prevents the temporary file from being
    // created, simulating a write failure before replacement.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let err = manager
        .create_entities(vec![entity("bob")])
        .await
        .unwrap_err();
    assert!(err.contains("Failed to write"), "{err}");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();

    // The previous valid target survives the failed rewrite.
    let graph = manager.read_graph().await.unwrap();
    assert_eq!(graph.entities.len(), 1, "previous graph preserved");
    assert_eq!(graph.entities[0].name, "alice");
    let leftovers: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[tokio::test]
async fn oversized_memory_file_is_rejected_before_unbounded_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.jsonl");
    tokio::fs::write(&path, vec![b'x'; MAX_MEMORY_FILE_BYTES + 1])
        .await
        .unwrap();
    let manager = KnowledgeGraphManager::new(path);
    let error = manager.read_graph().await.unwrap_err();
    assert!(error.contains("exceeds"));
}

#[tokio::test]
async fn mutation_inputs_are_bounded_and_nonempty() {
    let dir = tempfile::tempdir().unwrap();
    let manager = KnowledgeGraphManager::new(dir.path().join("memory.jsonl"));
    assert!(
        manager
            .create_entities(vec![Entity {
                name: String::new(),
                entity_type: "person".into(),
                observations: Vec::new(),
            }])
            .await
            .is_err()
    );
    assert!(
        manager
            .create_entities(vec![Entity {
                name: "a".repeat(MAX_MEMORY_TEXT_BYTES + 1),
                entity_type: "person".into(),
                observations: Vec::new(),
            }])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn persisted_graph_rejects_duplicate_entities() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.jsonl");
    let duplicate = entity("alice");
    let first = serde_json::to_string(&GraphItem::Entity(duplicate.clone())).unwrap();
    let second = serde_json::to_string(&GraphItem::Entity(duplicate)).unwrap();
    tokio::fs::write(&path, format!("{first}\n{second}"))
        .await
        .unwrap();

    let manager = KnowledgeGraphManager::new(path);
    let error = manager.read_graph().await.unwrap_err();
    assert!(error.contains("duplicate entity name"), "{error}");
}

#[tokio::test]
async fn persisted_graph_accepts_dangling_relations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.jsonl");
    let entity_line = serde_json::to_string(&GraphItem::Entity(entity("alice"))).unwrap();
    let relation_line = serde_json::to_string(&GraphItem::Relation(Relation {
        from: "alice".into(),
        to: "missing".into(),
        relation_type: "knows".into(),
    }))
    .unwrap();
    tokio::fs::write(&path, format!("{entity_line}\n{relation_line}"))
        .await
        .unwrap();

    let manager = KnowledgeGraphManager::new(path);
    let graph = manager.read_graph().await.unwrap();
    assert_eq!(graph.relations.len(), 1);
}

#[tokio::test]
async fn entity_creation_rejects_duplicate_initial_observations() {
    let dir = tempfile::tempdir().unwrap();
    let manager = KnowledgeGraphManager::new(dir.path().join("memory.jsonl"));
    let error = manager
        .create_entities(vec![Entity {
            name: "alice".into(),
            entity_type: "person".into(),
            observations: vec!["same".into(), "same".into()],
        }])
        .await
        .unwrap_err();
    assert!(error.contains("duplicate observations"), "{error}");
}

#[tokio::test]
async fn noop_mutations_do_not_persist() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.jsonl");
    let manager = KnowledgeGraphManager::new(path.clone());
    manager
        .create_entities(vec![entity("alice")])
        .await
        .unwrap();
    let dangling = Relation {
        from: "ghost".into(),
        to: "alice".into(),
        relation_type: "knows".into(),
    };
    let added = manager
        .create_relations(vec![dangling.clone()])
        .await
        .unwrap();
    assert_eq!(added.len(), 1);
    let before = tokio::fs::read(&path).await.unwrap();

    // All of these leave the graph identical and must not rewrite the file.
    let added = manager
        .create_entities(vec![entity("alice")])
        .await
        .unwrap();
    assert!(added.is_empty());
    let added = manager
        .create_relations(vec![dangling.clone()])
        .await
        .unwrap();
    assert!(added.is_empty(), "exact duplicate relation is a no-op");
    // "nobody" matches neither an entity nor a relation endpoint, so the
    // cascade delete cannot remove the dangling relation either.
    assert!(
        !manager
            .delete_entities(vec!["nobody".into()])
            .await
            .unwrap()
    );
    assert!(
        !manager
            .delete_relations(vec![Relation {
                from: "alice".into(),
                to: "ghost".into(),
                relation_type: "knows".into(),
            }])
            .await
            .unwrap()
    );

    let after = tokio::fs::read(&path).await.unwrap();
    assert_eq!(before, after, "no-op mutations must not rewrite the file");

    // A real mutation still persists.
    let added = manager.create_entities(vec![entity("bob")]).await.unwrap();
    assert_eq!(added.len(), 1);
    let graph = manager.read_graph().await.unwrap();
    assert_eq!(graph.entities.len(), 2);
}

#[tokio::test]
async fn noop_observation_mutations_do_not_persist() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.jsonl");
    let manager = KnowledgeGraphManager::new(path.clone());
    manager
        .create_entities(vec![Entity {
            name: "alice".into(),
            entity_type: "person".into(),
            observations: vec!["speaks Spanish".into()],
        }])
        .await
        .unwrap();
    let before = tokio::fs::read(&path).await.unwrap();

    let results = manager
        .add_observations(vec![ObservationInput {
            entity_name: "alice".into(),
            contents: vec!["speaks Spanish".into()],
        }])
        .await
        .unwrap();
    assert!(results[0].added_observations.is_empty());
    assert!(
        !manager
            .delete_observations(vec![super::super::ObservationDeletion {
                entity_name: "alice".into(),
                observations: vec!["nothing here".into()],
            }])
            .await
            .unwrap()
    );

    let after = tokio::fs::read(&path).await.unwrap();
    assert_eq!(
        before, after,
        "no-op observation mutations must not rewrite"
    );

    // Deleting a real observation reports the change and persists.
    assert!(
        manager
            .delete_observations(vec![super::super::ObservationDeletion {
                entity_name: "alice".into(),
                observations: vec!["speaks Spanish".into()],
            }])
            .await
            .unwrap()
    );
    let graph = manager.read_graph().await.unwrap();
    assert!(graph.entities[0].observations.is_empty());
}

/// A no-op mutation must succeed even when the persistence file could not be
/// written (read-only directory), proving that no write is attempted; a real
/// mutation in the same situation fails instead of silently losing data.
#[cfg(unix)]
#[tokio::test]
async fn noop_mutations_skip_writes_entirely_in_read_only_directories() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.jsonl");
    let manager = KnowledgeGraphManager::new(path.clone());
    manager
        .create_entities(vec![entity("alice")])
        .await
        .unwrap();

    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();

    // No-op: never reaches the write, so it succeeds in a read-only dir.
    let added = manager
        .create_entities(vec![entity("alice")])
        .await
        .unwrap();
    assert!(added.is_empty());
    assert!(!manager.delete_entities(vec!["ghost".into()]).await.unwrap());

    // Real change: the write is attempted and fails loudly.
    let error = manager
        .create_entities(vec![entity("bob")])
        .await
        .unwrap_err();
    assert!(error.contains("Failed to write"), "{error}");

    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let graph = manager.read_graph().await.unwrap();
    assert_eq!(graph.entities.len(), 1, "previous graph preserved");
}
