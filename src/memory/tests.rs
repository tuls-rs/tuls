//! Server-level tests: notification discipline and change reporting.
//!
//! Subscribers (`resources/subscribe` + `resources/listen`) must receive a
//! graph-updated notification only when a mutation actually changed the
//! graph, and mutation results must report whether anything changed.

use super::*;

use rmcp::handler::server::wrapper::Parameters;

fn entity(name: &str) -> Entity {
    Entity {
        name: name.to_string(),
        entity_type: "person".to_string(),
        observations: Vec::new(),
    }
}

fn server_with(manager: KnowledgeGraphManager) -> MemoryServer {
    MemoryServer::new(
        manager,
        ToolPolicy::from_selectors(&[], &[], TOOL_SPECS).unwrap(),
    )
}

#[tokio::test]
async fn notifications_fire_only_on_real_changes() {
    let dir = tempfile::tempdir().unwrap();
    let server = server_with(KnowledgeGraphManager::new(dir.path().join("memory.jsonl")));
    let mut rx = server.notify_tx.subscribe();

    // A real change notifies.
    server
        .create_entities(Parameters(CreateEntitiesArgs {
            entities: vec![entity("alice")],
        }))
        .await
        .expect("create alice");
    assert!(
        rx.try_recv().is_ok(),
        "a real mutation must notify subscribers"
    );

    // A duplicate create is a no-op and must not notify.
    server
        .create_entities(Parameters(CreateEntitiesArgs {
            entities: vec![entity("alice")],
        }))
        .await
        .expect("duplicate create is still a success");
    assert!(
        matches!(
            rx.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ),
        "a no-op mutation must not notify subscribers"
    );

    // Deleting a non-existent entity is a no-op and must not notify.
    server
        .delete_entities(Parameters(DeleteEntitiesArgs {
            entity_names: vec!["ghost".into()],
        }))
        .await
        .expect("unmatched delete is still a success");
    assert!(
        matches!(
            rx.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ),
        "an unmatched delete must not notify subscribers"
    );

    // Deleting a real entity notifies again.
    server
        .delete_entities(Parameters(DeleteEntitiesArgs {
            entity_names: vec!["alice".into()],
        }))
        .await
        .expect("delete alice");
    assert!(
        rx.try_recv().is_ok(),
        "a real delete must notify subscribers"
    );
}

#[tokio::test]
async fn create_results_report_exactly_what_changed() {
    let dir = tempfile::tempdir().unwrap();
    let server = server_with(KnowledgeGraphManager::new(dir.path().join("memory.jsonl")));

    let result = server
        .create_entities(Parameters(CreateEntitiesArgs {
            entities: vec![entity("alice")],
        }))
        .await
        .expect("create alice");
    let structured = result.structured_content.expect("structured result");
    assert_eq!(structured["entities"].as_array().map(Vec::len), Some(1));

    let result = server
        .create_entities(Parameters(CreateEntitiesArgs {
            entities: vec![entity("alice"), entity("bob")],
        }))
        .await
        .expect("create alice again");
    let structured = result.structured_content.expect("structured result");
    let added = structured["entities"].as_array().expect("entities array");
    assert_eq!(added.len(), 1, "only bob was actually added: {added:?}");
    assert_eq!(added[0]["name"], "bob");
}

#[tokio::test]
async fn delete_results_report_change_and_honest_messages() {
    let dir = tempfile::tempdir().unwrap();
    let server = server_with(KnowledgeGraphManager::new(dir.path().join("memory.jsonl")));
    server
        .create_entities(Parameters(CreateEntitiesArgs {
            entities: vec![entity("alice")],
        }))
        .await
        .expect("create alice");

    let result = server
        .delete_entities(Parameters(DeleteEntitiesArgs {
            entity_names: vec!["ghost".into()],
        }))
        .await
        .expect("unmatched delete");
    let structured = result.structured_content.expect("structured result");
    assert_eq!(structured["changed"], false);
    assert!(
        structured["message"]
            .as_str()
            .is_some_and(|message| message.contains("nothing was deleted")),
        "{structured}"
    );

    let result = server
        .delete_entities(Parameters(DeleteEntitiesArgs {
            entity_names: vec!["alice".into()],
        }))
        .await
        .expect("real delete");
    let structured = result.structured_content.expect("structured result");
    assert_eq!(structured["changed"], true);
    assert_eq!(structured["message"], "Entities deleted successfully");
}

#[tokio::test]
async fn observation_mutations_notify_only_when_an_observation_is_added() {
    let dir = tempfile::tempdir().unwrap();
    let server = server_with(KnowledgeGraphManager::new(dir.path().join("memory.jsonl")));
    server
        .create_entities(Parameters(CreateEntitiesArgs {
            entities: vec![entity("alice")],
        }))
        .await
        .expect("create alice");
    let mut rx = server.notify_tx.subscribe();

    server
        .add_observations(Parameters(AddObservationsArgs {
            observations: vec![ObservationInput {
                entity_name: "alice".into(),
                contents: vec!["speaks Spanish".into()],
            }],
        }))
        .await
        .expect("add observation");
    assert!(
        rx.try_recv().is_ok(),
        "adding a new observation must notify"
    );

    server
        .add_observations(Parameters(AddObservationsArgs {
            observations: vec![ObservationInput {
                entity_name: "alice".into(),
                contents: vec!["speaks Spanish".into()],
            }],
        }))
        .await
        .expect("duplicate observation");
    assert!(
        matches!(
            rx.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ),
        "a duplicate observation must not notify"
    );
}
