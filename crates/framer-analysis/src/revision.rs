use std::fmt;

use framer_core::{BuildingModel, ProjectError, save_project};

/// Version of the fact/evaluator contract included in every graph revision fingerprint.
/// Increment when the same canonical authored model would produce meaningfully different graph
/// semantics without a project-schema change.
pub const GRAPH_CONTRACT_VERSION: u32 = 2;
const REVISION_DOMAIN: &[u8] = b"framer.analysis.graph-revision\0";

/// Deterministic fingerprint of the canonical post-propagation authored model, external analysis
/// inputs, and analysis contract. This is separate from the app's process-local
/// `document_revision`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GraphRevision([u8; 32]);

impl GraphRevision {
    pub fn for_model(model: &BuildingModel) -> Result<Self, ProjectError> {
        Self::for_model_with_inputs(
            model,
            GRAPH_CONTRACT_VERSION,
            &current_library_source_fingerprint(),
        )
    }

    #[cfg(test)]
    pub(crate) fn for_model_with_contract(
        model: &BuildingModel,
        contract_version: u32,
    ) -> Result<Self, ProjectError> {
        Self::for_model_with_inputs(
            model,
            contract_version,
            &current_library_source_fingerprint(),
        )
    }

    fn for_model_with_inputs(
        model: &BuildingModel,
        contract_version: u32,
        library_source_fingerprint: &[u8],
    ) -> Result<Self, ProjectError> {
        let canonical = save_project(model)?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(REVISION_DOMAIN);
        hasher.update(&contract_version.to_le_bytes());
        hasher.update(&(library_source_fingerprint.len() as u64).to_le_bytes());
        hasher.update(library_source_fingerprint);
        hasher.update(canonical.as_bytes());
        Ok(Self(*hasher.finalize().as_bytes()))
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

fn current_library_source_fingerprint() -> Vec<u8> {
    match framer_library::starter_library_ref() {
        Ok(loaded) => {
            let mut fingerprint = b"available\0".to_vec();
            fingerprint.extend_from_slice(loaded.content_hash.as_bytes());
            fingerprint
        }
        // Lifecycle lowering intentionally preserves the pre-analysis app behavior when the
        // bundled source is unavailable. The availability discriminator still keeps cache
        // authority separate from a successful library load.
        Err(_) => b"unavailable\0".to_vec(),
    }
}

impl fmt::Display for GraphRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", blake3::Hash::from_bytes(self.0).to_hex())
    }
}

#[cfg(test)]
mod tests {
    use framer_core::BuildingModel;

    use super::*;

    #[test]
    fn canonical_reordering_does_not_change_revision() {
        let first = BuildingModel::demo_shell();
        let mut reordered = first.clone();
        reordered.walls.reverse();
        reordered.materials.reverse();
        for wall in &mut reordered.walls {
            wall.openings.reverse();
        }

        assert_eq!(
            GraphRevision::for_model(&first).unwrap(),
            GraphRevision::for_model(&reordered).unwrap()
        );
    }

    #[test]
    fn semantic_stack_order_and_contract_version_change_revision() {
        let first = BuildingModel::demo_shell();
        let mut reordered_stack = first.clone();
        let duplicate = reordered_stack.standards_packs[0].clone();
        let mut second_pack = duplicate;
        second_pack.id = framer_core::ElementId::new("standards-later");
        second_pack.name = "Later pack".to_owned();
        reordered_stack.standards_packs.push(second_pack.clone());
        reordered_stack.standards.push(second_pack.id.clone());
        reordered_stack.validate().unwrap();
        let normal = GraphRevision::for_model(&reordered_stack).unwrap();
        reordered_stack.standards.reverse();
        let reversed = GraphRevision::for_model(&reordered_stack).unwrap();

        assert_ne!(normal, reversed);
        assert_ne!(
            GraphRevision::for_model_with_contract(&first, GRAPH_CONTRACT_VERSION).unwrap(),
            GraphRevision::for_model_with_contract(&first, GRAPH_CONTRACT_VERSION + 1).unwrap()
        );
    }

    #[test]
    fn external_library_source_changes_revision_for_the_same_model() {
        let model = BuildingModel::new();
        let first = GraphRevision::for_model_with_inputs(
            &model,
            GRAPH_CONTRACT_VERSION,
            b"available\0blake3:first",
        )
        .unwrap();
        let second = GraphRevision::for_model_with_inputs(
            &model,
            GRAPH_CONTRACT_VERSION,
            b"available\0blake3:second",
        )
        .unwrap();

        assert_ne!(first, second);
    }
}
