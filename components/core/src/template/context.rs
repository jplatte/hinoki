use std::{collections::BTreeMap, sync::Arc};

use serde::{Serialize, Serializer};

use crate::content::{DirectoryMetadata, FileMetadata};

#[derive(Debug)]
pub(crate) struct HinokiContext {
    pub syntax_highlight_theme: Option<String>,
    pub current_dir_subdirs: Arc<BTreeMap<String, DirectoryMetadata>>,
}

impl minijinja::value::Object for HinokiContext {
    fn repr(self: &Arc<Self>) -> minijinja::value::ObjectRepr {
        minijinja::value::ObjectRepr::Plain
    }
}

#[derive(Serialize)]
pub(crate) struct TemplateContext<'a> {
    pub content: String,
    pub page: &'a FileMetadata,
    pub config: minijinja::Value,
    #[serde(rename = "$hinoki_cx", serialize_with = "serialize_hinoki_cx")]
    pub hinoki_cx: Arc<HinokiContext>,
}

fn serialize_hinoki_cx<S: Serializer>(
    cx: &Arc<HinokiContext>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    minijinja::Value::from(minijinja::value::DynObject::new(cx.clone())).serialize(serializer)
}
