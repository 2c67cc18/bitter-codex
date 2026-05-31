use std::collections::BTreeMap;

use crate::context::AdditionalContextDeveloperFragment;
use crate::context::AdditionalContextUserFragment;
use crate::context::ContextualUserFragment;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::protocol::AdditionalContextEntry;
use codex_protocol::protocol::AdditionalContextKind;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AdditionalContextStore {
    values: BTreeMap<String, AdditionalContextEntry>,
}

impl AdditionalContextStore {
    pub(crate) fn merge(
        &mut self,
        values: BTreeMap<String, AdditionalContextEntry>,
    ) -> Vec<ResponseInputItem> {
        let fragments = values
            .iter()
            .filter(|(key, value)| self.values.get(*key) != Some(*value))
            .map(|(key, entry)| match entry.kind {
                AdditionalContextKind::Untrusted => {
                    AdditionalContextUserFragment::new(key.clone(), entry.value.clone())
                        .into_response_input_item()
                }
                AdditionalContextKind::Application => {
                    AdditionalContextDeveloperFragment::new(key.clone(), entry.value.clone())
                        .into_response_input_item()
                }
            })
            .collect();
        self.values = values;
        fragments
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn entry(value: &str, kind: AdditionalContextKind) -> AdditionalContextEntry {
        AdditionalContextEntry {
            value: value.to_string(),
            kind,
        }
    }

    fn input_texts(items: &[ResponseInputItem]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|item| match item {
                ResponseInputItem::Message { role, content, .. } => {
                    let text = content
                        .iter()
                        .map(|content| match content {
                            codex_protocol::models::ContentItem::InputText { text } => text.clone(),
                            _ => String::new(),
                        })
                        .collect::<String>();
                    (role.clone(), text)
                }
                _ => (String::new(), String::new()),
            })
            .collect()
    }

    #[test]
    fn merge_renders_new_context_and_dedupes_unchanged_values() {
        let mut store = AdditionalContextStore::default();
        let values = BTreeMap::from([
            (
                "selection".to_string(),
                entry("selected text", AdditionalContextKind::Untrusted),
            ),
            (
                "app".to_string(),
                entry("state", AdditionalContextKind::Application),
            ),
        ]);

        let first = store.merge(values.clone());
        let second = store.merge(values);

        assert_eq!(
            input_texts(&first),
            vec![
                ("developer".to_string(), "<app>state</app>".to_string()),
                (
                    "user".to_string(),
                    "<external_selection>selected text</external_selection>".to_string()
                ),
            ]
        );
        assert_eq!(second, Vec::new());
    }

    #[test]
    fn merge_readds_context_after_deletion() {
        let mut store = AdditionalContextStore::default();
        let values = BTreeMap::from([(
            "selection".to_string(),
            entry("selected text", AdditionalContextKind::Untrusted),
        )]);

        assert_eq!(store.merge(values.clone()).len(), 1);
        assert_eq!(store.merge(BTreeMap::new()), Vec::new());

        let readded = store.merge(values);

        assert_eq!(
            input_texts(&readded),
            vec![(
                "user".to_string(),
                "<external_selection>selected text</external_selection>".to_string()
            )]
        );
    }
}
