use std::{collections::BTreeSet, path::PathBuf};

use serde::{Deserialize, Serialize};
use tokio::{io::AsyncReadExt, sync::Mutex};

use crate::support::atomic_write;

const MAX_MEMORY_FILE_BYTES: usize = 8 * 1024 * 1024;
const MAX_MUTATION_BATCH_ITEMS: usize = 1_024;
const MAX_GRAPH_ENTITIES: usize = 100_000;
const MAX_GRAPH_RELATIONS: usize = 200_000;
const MAX_ENTITY_OBSERVATIONS: usize = 4_096;
const MAX_MEMORY_TEXT_BYTES: usize = 16 * 1024;

/// An entity (node) in the knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "An entity in the knowledge graph")]
pub struct Entity {
    /// The name of the entity
    #[schemars(description = "The name of the entity")]
    pub name: String,
    /// The type of the entity
    #[schemars(description = "The type of the entity")]
    pub entity_type: String,
    /// An array of observation contents associated with the entity
    #[schemars(description = "An array of observation contents associated with the entity")]
    pub observations: Vec<String>,
}

/// A directed relation between two entities.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "A directed relation between two entities")]
pub struct Relation {
    /// The name of the entity where the relation starts
    #[schemars(description = "The name of the entity where the relation starts")]
    pub from: String,
    /// The name of the entity where the relation ends
    #[schemars(description = "The name of the entity where the relation ends")]
    pub to: String,
    /// The type of the relation
    #[schemars(description = "The type of the relation")]
    pub relation_type: String,
}

/// The full knowledge graph.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
#[schemars(description = "The knowledge graph")]
pub struct KnowledgeGraph {
    pub entities: Vec<Entity>,
    pub relations: Vec<Relation>,
}

/// Input for `add_observations`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Observations to add to an entity")]
pub struct ObservationInput {
    /// The name of the entity to add the observations to
    #[schemars(description = "The name of the entity to add the observations to")]
    pub entity_name: String,
    /// An array of observation contents to add
    #[schemars(description = "An array of observation contents to add")]
    pub contents: Vec<String>,
}

/// Result of `add_observations`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "The result of adding observations to an entity")]
pub struct AddedObservation {
    pub entity_name: String,
    pub added_observations: Vec<String>,
}

/// One line in the JSONL storage file.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum GraphItem {
    Entity(Entity),
    Relation(Relation),
}

/// Persists the knowledge graph as JSONL. Mutations are serialized, load the
/// current graph, apply one validated change, and atomically rewrite the file.
pub struct KnowledgeGraphManager {
    path: PathBuf,
    gate: Mutex<()>,
}

impl KnowledgeGraphManager {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            gate: Mutex::new(()),
        }
    }

    async fn load(&self) -> Result<KnowledgeGraph, String> {
        let file = match tokio::fs::File::open(&self.path).await {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(KnowledgeGraph::default());
            }
            Err(error) => {
                return Err(format!("Failed to read {}: {error}", self.path.display()));
            }
        };
        let mut bytes = Vec::new();
        let mut limited = file.take((MAX_MEMORY_FILE_BYTES + 1) as u64);
        limited
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| format!("Failed to read {}: {error}", self.path.display()))?;
        if bytes.len() > MAX_MEMORY_FILE_BYTES {
            return Err(format!(
                "Memory file exceeds the {} byte limit",
                MAX_MEMORY_FILE_BYTES
            ));
        }
        let data = String::from_utf8(bytes)
            .map_err(|_| format!("Memory file {} is not valid UTF-8", self.path.display()))?;

        let mut graph = KnowledgeGraph::default();
        for line in data.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let item: GraphItem = serde_json::from_str(line)
                .map_err(|e| format!("Corrupt memory file {}: {e}", self.path.display()))?;
            match item {
                GraphItem::Entity(entity) => graph.entities.push(entity),
                GraphItem::Relation(relation) => graph.relations.push(relation),
            }
        }
        validate_graph(&graph)
            .map_err(|error| format!("Invalid memory file {}: {error}", self.path.display()))?;
        Ok(graph)
    }

    async fn save(&self, graph: &KnowledgeGraph) -> Result<(), String> {
        validate_graph(graph)?;
        let mut contents = String::new();
        for item in graph
            .entities
            .iter()
            .cloned()
            .map(GraphItem::Entity)
            .chain(graph.relations.iter().cloned().map(GraphItem::Relation))
        {
            let line = serde_json::to_string(&item)
                .map_err(|error| format!("Failed to serialize memory item: {error}"))?;
            let separator = if contents.is_empty() { 0 } else { 1 };
            if contents
                .len()
                .saturating_add(separator)
                .saturating_add(line.len())
                > MAX_MEMORY_FILE_BYTES
            {
                return Err(format!(
                    "Knowledge graph exceeds the {} byte storage limit",
                    MAX_MEMORY_FILE_BYTES
                ));
            }
            if separator != 0 {
                contents.push('\n');
            }
            contents.push_str(&line);
        }
        atomic_write(&self.path, contents.as_bytes())
            .await
            .map_err(|e| format!("Failed to write {}: {e}", self.path.display()))
    }

    /// Persist only when a mutation actually changed the graph. No-op
    /// mutations (duplicates, unmatched deletes) must neither rewrite the
    /// file nor report a change to subscribers.
    async fn save_if_changed(&self, graph: &KnowledgeGraph, changed: bool) -> Result<(), String> {
        if changed {
            self.save(graph).await?;
        }
        Ok(())
    }

    /// Create entities, ignoring names that already exist. Returns the
    /// entities that were actually added; an empty result means nothing
    /// changed and the persistence file is left untouched.
    pub async fn create_entities(&self, entities: Vec<Entity>) -> Result<Vec<Entity>, String> {
        validate_batch_len(entities.len())?;
        for entity in &entities {
            validate_entity(entity)?;
        }
        let _guard = self.gate.lock().await;
        let mut graph = self.load().await?;
        let mut names: BTreeSet<String> = graph
            .entities
            .iter()
            .map(|entity| entity.name.clone())
            .collect();
        let new_entities: Vec<Entity> = entities
            .into_iter()
            .filter(|entity| names.insert(entity.name.clone()))
            .collect();
        graph.entities.extend(new_entities.clone());
        self.save_if_changed(&graph, !new_entities.is_empty())
            .await?;
        Ok(new_entities)
    }

    /// Create relations, skipping exact duplicates. Returns the relations
    /// that were actually added; an empty result means nothing changed and
    /// the persistence file is left untouched.
    pub async fn create_relations(
        &self,
        relations: Vec<Relation>,
    ) -> Result<Vec<Relation>, String> {
        validate_batch_len(relations.len())?;
        for relation in &relations {
            validate_relation(relation)?;
        }
        let _guard = self.gate.lock().await;
        let mut graph = self.load().await?;
        let mut existing: BTreeSet<(String, String, String)> = graph
            .relations
            .iter()
            .map(|relation| {
                (
                    relation.from.clone(),
                    relation.to.clone(),
                    relation.relation_type.clone(),
                )
            })
            .collect();
        let new_relations: Vec<Relation> = relations
            .into_iter()
            .filter(|relation| {
                existing.insert((
                    relation.from.clone(),
                    relation.to.clone(),
                    relation.relation_type.clone(),
                ))
            })
            .collect();
        graph.relations.extend(new_relations.clone());
        self.save_if_changed(&graph, !new_relations.is_empty())
            .await?;
        Ok(new_relations)
    }

    /// Add observations to entities. Fails if any entity does not exist.
    /// Returns what was actually added; when every observation is a
    /// duplicate nothing changes and the persistence file is untouched.
    pub async fn add_observations(
        &self,
        observations: Vec<ObservationInput>,
    ) -> Result<Vec<AddedObservation>, String> {
        validate_batch_len(observations.len())?;
        for input in &observations {
            validate_text("entity name", &input.entity_name)?;
            validate_batch_len(input.contents.len())?;
            for content in &input.contents {
                validate_text("observation", content)?;
            }
        }
        let _guard = self.gate.lock().await;
        let mut graph = self.load().await?;
        let mut results = Vec::with_capacity(observations.len());
        let mut changed = false;
        for input in observations {
            let entity = graph
                .entities
                .iter_mut()
                .find(|e| e.name == input.entity_name)
                .ok_or_else(|| format!("Entity with name {} not found", input.entity_name))?;
            let mut existing: BTreeSet<String> = entity.observations.iter().cloned().collect();
            let new_observations: Vec<String> = input
                .contents
                .into_iter()
                .filter(|content| existing.insert(content.clone()))
                .collect();
            changed |= !new_observations.is_empty();
            entity.observations.extend(new_observations.clone());
            results.push(AddedObservation {
                entity_name: entity.name.clone(),
                added_observations: new_observations,
            });
        }
        self.save_if_changed(&graph, changed).await?;
        Ok(results)
    }

    /// Delete entities and cascade-delete their relations. Returns whether
    /// anything was actually removed; a delete that matches nothing leaves
    /// the persistence file untouched.
    pub async fn delete_entities(&self, entity_names: Vec<String>) -> Result<bool, String> {
        validate_batch_len(entity_names.len())?;
        for name in &entity_names {
            validate_text("entity name", name)?;
        }
        let _guard = self.gate.lock().await;
        let mut graph = self.load().await?;
        let entity_names: BTreeSet<String> = entity_names.into_iter().collect();
        let entities_before = graph.entities.len();
        let relations_before = graph.relations.len();
        graph
            .entities
            .retain(|entity| !entity_names.contains(&entity.name));
        graph.relations.retain(|relation| {
            !entity_names.contains(&relation.from) && !entity_names.contains(&relation.to)
        });
        let changed =
            graph.entities.len() != entities_before || graph.relations.len() != relations_before;
        self.save_if_changed(&graph, changed).await?;
        Ok(changed)
    }

    /// Delete specific observations from entities. Missing entities or
    /// observations are silently ignored. Returns whether anything was
    /// actually removed.
    pub async fn delete_observations(
        &self,
        deletions: Vec<super::ObservationDeletion>,
    ) -> Result<bool, String> {
        validate_batch_len(deletions.len())?;
        for deletion in &deletions {
            validate_text("entity name", &deletion.entity_name)?;
            validate_batch_len(deletion.observations.len())?;
            for observation in &deletion.observations {
                validate_text("observation", observation)?;
            }
        }
        let _guard = self.gate.lock().await;
        let mut graph = self.load().await?;
        let mut changed = false;
        for deletion in deletions {
            if let Some(entity) = graph
                .entities
                .iter_mut()
                .find(|e| e.name == deletion.entity_name)
            {
                let before = entity.observations.len();
                entity
                    .observations
                    .retain(|o| !deletion.observations.contains(o));
                changed |= entity.observations.len() != before;
            }
        }
        self.save_if_changed(&graph, changed).await?;
        Ok(changed)
    }

    /// Delete specific relations. Missing relations are silently ignored.
    /// Returns whether anything was actually removed.
    pub async fn delete_relations(&self, relations: Vec<Relation>) -> Result<bool, String> {
        validate_batch_len(relations.len())?;
        for relation in &relations {
            validate_relation(relation)?;
        }
        let _guard = self.gate.lock().await;
        let mut graph = self.load().await?;
        let before = graph.relations.len();
        graph.relations.retain(|r| {
            !relations.iter().any(|del| {
                r.from == del.from && r.to == del.to && r.relation_type == del.relation_type
            })
        });
        let changed = graph.relations.len() != before;
        self.save_if_changed(&graph, changed).await?;
        Ok(changed)
    }

    /// Read the entire knowledge graph.
    pub async fn read_graph(&self) -> Result<KnowledgeGraph, String> {
        let _guard = self.gate.lock().await;
        self.load().await
    }

    /// Search for entities whose name, type, or observations contain the
    /// query (case-insensitive). Relations with at least one matching
    /// endpoint are included, so callers can discover connections to nodes
    /// outside the result set.
    pub async fn search_nodes(&self, query: &str) -> Result<KnowledgeGraph, String> {
        validate_text("search query", query)?;
        let _guard = self.gate.lock().await;
        let graph = self.load().await?;
        let query = query.to_lowercase();

        let filtered_entities: Vec<Entity> = graph
            .entities
            .into_iter()
            .filter(|e| {
                e.name.to_lowercase().contains(&query)
                    || e.entity_type.to_lowercase().contains(&query)
                    || e.observations
                        .iter()
                        .any(|o| o.to_lowercase().contains(&query))
            })
            .collect();

        let filtered_names: BTreeSet<&str> = filtered_entities
            .iter()
            .map(|entity| entity.name.as_str())
            .collect();
        let filtered_relations: Vec<Relation> = graph
            .relations
            .into_iter()
            .filter(|r| {
                filtered_names.contains(r.from.as_str()) || filtered_names.contains(r.to.as_str())
            })
            .collect();

        Ok(KnowledgeGraph {
            entities: filtered_entities,
            relations: filtered_relations,
        })
    }

    /// Open specific entities by name. Relations with at least one endpoint
    /// in the requested set are included. Non-existent names are skipped.
    pub async fn open_nodes(&self, names: Vec<String>) -> Result<KnowledgeGraph, String> {
        validate_batch_len(names.len())?;
        for name in &names {
            validate_text("entity name", name)?;
        }
        let _guard = self.gate.lock().await;
        let graph = self.load().await?;
        let names: BTreeSet<String> = names.into_iter().collect();

        let filtered_entities: Vec<Entity> = graph
            .entities
            .into_iter()
            .filter(|entity| names.contains(&entity.name))
            .collect();

        let filtered_names: BTreeSet<&str> = filtered_entities
            .iter()
            .map(|entity| entity.name.as_str())
            .collect();
        let filtered_relations: Vec<Relation> = graph
            .relations
            .into_iter()
            .filter(|r| {
                filtered_names.contains(r.from.as_str()) || filtered_names.contains(r.to.as_str())
            })
            .collect();

        Ok(KnowledgeGraph {
            entities: filtered_entities,
            relations: filtered_relations,
        })
    }
}

fn validate_batch_len(len: usize) -> Result<(), String> {
    if len > MAX_MUTATION_BATCH_ITEMS {
        Err(format!(
            "mutation batch exceeds the {} item limit",
            MAX_MUTATION_BATCH_ITEMS
        ))
    } else {
        Ok(())
    }
}

fn validate_text(kind: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{kind} must not be empty"));
    }
    if value.len() > MAX_MEMORY_TEXT_BYTES {
        return Err(format!(
            "{kind} exceeds the {} byte limit",
            MAX_MEMORY_TEXT_BYTES
        ));
    }
    Ok(())
}

fn validate_entity(entity: &Entity) -> Result<(), String> {
    validate_text("entity name", &entity.name)?;
    validate_text("entity type", &entity.entity_type)?;
    if entity.observations.len() > MAX_ENTITY_OBSERVATIONS {
        return Err(format!(
            "entity observations exceed the {MAX_ENTITY_OBSERVATIONS} item limit"
        ));
    }
    let mut observations = BTreeSet::new();
    for observation in &entity.observations {
        validate_text("observation", observation)?;
        if !observations.insert(observation.as_str()) {
            return Err(format!(
                "entity {:?} contains duplicate observations",
                entity.name
            ));
        }
    }
    Ok(())
}

fn validate_relation(relation: &Relation) -> Result<(), String> {
    validate_text("relation source", &relation.from)?;
    validate_text("relation target", &relation.to)?;
    validate_text("relation type", &relation.relation_type)
}

fn validate_graph(graph: &KnowledgeGraph) -> Result<(), String> {
    if graph.entities.len() > MAX_GRAPH_ENTITIES {
        return Err(format!(
            "knowledge graph exceeds the {MAX_GRAPH_ENTITIES} entity limit"
        ));
    }
    if graph.relations.len() > MAX_GRAPH_RELATIONS {
        return Err(format!(
            "knowledge graph exceeds the {MAX_GRAPH_RELATIONS} relation limit"
        ));
    }

    let mut names = BTreeSet::new();
    for entity in &graph.entities {
        validate_entity(entity)?;
        if !names.insert(entity.name.as_str()) {
            return Err(format!("duplicate entity name {:?}", entity.name));
        }
    }

    let mut relations = BTreeSet::new();
    for relation in &graph.relations {
        validate_relation(relation)?;
        if !relations.insert((
            relation.from.as_str(),
            relation.to.as_str(),
            relation.relation_type.as_str(),
        )) {
            return Err(format!(
                "duplicate relation: {} -> {} ({})",
                relation.from, relation.to, relation.relation_type
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
