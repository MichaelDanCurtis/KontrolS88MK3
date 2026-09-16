//! Newtype identifiers used across the graph.

use serde::{Deserialize, Serialize};

macro_rules! id_newtype {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_newtype!(NodeId);
id_newtype!(DeviceId);
id_newtype!(ParamName);
id_newtype!(BindingId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_distinct_types_with_string_access() {
        let n = NodeId::new("lfo1");
        assert_eq!(n.as_str(), "lfo1");
        assert_eq!(n, NodeId::new("lfo1"));
        assert_ne!(NodeId::new("a"), NodeId::new("b"));
    }
}
