//! Semantic device models: named parameters mapped to wire encodings.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PackError {
    #[error("failed to parse pack yaml: {0}")]
    Yaml(#[from] serde_norway::Error),
    #[error("parameter {name} has invalid range: min {min} max {max}")]
    InvalidRange { name: String, min: f32, max: f32 },
    #[error("parameter {name} has out-of-range cc number {cc} (must be 0-127)")]
    InvalidCc { name: String, cc: u8 },
    #[error(
        "parameter {name} uses 14-bit cc {msb_cc}, whose LSB would land on cc {lsb} (exceeds 127); msb_cc must be <= 95"
    )]
    Cc14LsbOverflow { name: String, msb_cc: u8, lsb: u16 },
    #[error("pack has no parameters")]
    NoParameters,
    #[error("duplicate device pack name {0:?}")]
    DuplicatePack(String),
    #[error("unknown device pack {name:?}; available: {available:?}")]
    UnknownPack {
        name: String,
        available: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    /// Single 7-bit control change.
    Cc7 { cc: u8 },
    /// 14-bit pair: MSB on `msb_cc`, LSB on `msb_cc + 32`.
    Cc14 { msb_cc: u8 },
    /// Non-registered parameter number, 14-bit data entry.
    Nrpn { msb: u8, lsb: u8 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub encoding: Encoding,
    pub min: f32,
    pub max: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DevicePack {
    pub name: String,
    pub display_name: String,
    pub parameters: HashMap<String, Parameter>,
}

impl DevicePack {
    pub fn from_yaml(src: &str) -> Result<Self, PackError> {
        let pack: DevicePack = serde_norway::from_str(src)?;
        pack.validate()?;
        Ok(pack)
    }

    pub fn parameter(&self, name: &str) -> Option<&Parameter> {
        self.parameters.get(name)
    }

    fn validate(&self) -> Result<(), PackError> {
        if self.parameters.is_empty() {
            return Err(PackError::NoParameters);
        }
        for (name, p) in &self.parameters {
            if !p.min.is_finite() || !p.max.is_finite() || p.min >= p.max {
                return Err(PackError::InvalidRange {
                    name: name.clone(),
                    min: p.min,
                    max: p.max,
                });
            }
            let ccs: &[u8] = match &p.encoding {
                Encoding::Cc7 { cc } => &[*cc],
                Encoding::Cc14 { msb_cc } => &[*msb_cc],
                Encoding::Nrpn { msb, lsb } => &[*msb, *lsb],
            };
            for &cc in ccs {
                if cc > 127 {
                    return Err(PackError::InvalidCc {
                        name: name.clone(),
                        cc,
                    });
                }
            }
            // A 14-bit CC pair puts the LSB at cc+32, which must also fit.
            if let Encoding::Cc14 { msb_cc } = &p.encoding
                && *msb_cc > 95
            {
                let lsb = (*msb_cc as u16) + 32;
                return Err(PackError::Cc14LsbOverflow {
                    name: name.clone(),
                    msb_cc: *msb_cc,
                    lsb,
                });
            }
        }
        Ok(())
    }
}

/// Validated device packs keyed by their stable pack name.
///
/// Runtime device IDs (for example, `peak-left`) are deliberately separate
/// from pack names (for example, `novation-peak`), so multiple physical
/// instruments can share the same semantic model.
#[derive(Debug, Clone, Default)]
pub struct PackRegistry {
    packs: HashMap<String, DevicePack>,
}

impl PackRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_packs(packs: impl IntoIterator<Item = DevicePack>) -> Result<Self, PackError> {
        let mut registry = Self::new();
        for pack in packs {
            registry.insert(pack)?;
        }
        Ok(registry)
    }

    pub fn insert(&mut self, pack: DevicePack) -> Result<(), PackError> {
        let name = pack.name.clone();
        if self.packs.contains_key(&name) {
            return Err(PackError::DuplicatePack(name));
        }
        self.packs.insert(name, pack);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Result<&DevicePack, PackError> {
        self.packs.get(name).ok_or_else(|| PackError::UnknownPack {
            name: name.to_owned(),
            available: self.names().map(str::to_owned).collect(),
        })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        let mut names: Vec<&str> = self.packs.keys().map(String::as_str).collect();
        names.sort_unstable();
        names.into_iter()
    }

    pub fn len(&self) -> usize {
        self.packs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.packs.is_empty()
    }
}

/// The Peak pack shipped with the binary.
pub const NOVATION_PEAK_YAML: &str = include_str!("../packs/novation-peak.yaml");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_peak_pack_loads_and_validates() {
        let pack = DevicePack::from_yaml(NOVATION_PEAK_YAML).expect("shipped pack must load");
        assert_eq!(pack.name, "novation-peak");
        assert_eq!(pack.display_name, "Novation Peak");
    }

    #[test]
    fn peak_pack_exposes_documented_cc_numbers() {
        let pack = DevicePack::from_yaml(NOVATION_PEAK_YAML).unwrap();

        let cutoff = pack.parameter("filter.cutoff").expect("cutoff present");
        assert_eq!(cutoff.encoding, Encoding::Cc7 { cc: 29 });

        let resonance = pack
            .parameter("filter.resonance")
            .expect("resonance present");
        assert_eq!(resonance.encoding, Encoding::Cc7 { cc: 79 });

        let attack = pack.parameter("amp_env.attack").expect("attack present");
        assert_eq!(attack.encoding, Encoding::Cc7 { cc: 86 });

        let decay = pack.parameter("amp_env.decay").expect("decay present");
        assert_eq!(decay.encoding, Encoding::Cc7 { cc: 87 });

        let sustain = pack.parameter("amp_env.sustain").expect("sustain present");
        assert_eq!(sustain.encoding, Encoding::Cc7 { cc: 88 });

        let release = pack.parameter("amp_env.release").expect("release present");
        assert_eq!(release.encoding, Encoding::Cc7 { cc: 89 });

        let lfo1_rate = pack.parameter("lfo1.rate").expect("lfo1.rate present");
        assert_eq!(lfo1_rate.encoding, Encoding::Cc7 { cc: 30 });
    }

    #[test]
    fn unknown_parameter_returns_none() {
        let pack = DevicePack::from_yaml(NOVATION_PEAK_YAML).unwrap();
        assert!(pack.parameter("no.such.param").is_none());
    }

    #[test]
    fn rejects_inverted_range() {
        let src = r#"
name: bad
display_name: Bad
parameters:
  a.b:
    encoding: !cc7 { cc: 1 }
    min: 10.0
    max: 1.0
"#;
        assert!(matches!(
            DevicePack::from_yaml(src),
            Err(PackError::InvalidRange { .. })
        ));
    }

    #[test]
    fn rejects_cc14_whose_lsb_would_overflow() {
        let src = r#"
name: bad
display_name: Bad
parameters:
  a.b:
    encoding: !cc14 { msb_cc: 100 }
    min: 0.0
    max: 1.0
"#;
        assert!(matches!(
            DevicePack::from_yaml(src),
            Err(PackError::Cc14LsbOverflow { .. })
        ));
    }

    #[test]
    fn rejects_pack_with_no_parameters() {
        let src = "name: empty\ndisplay_name: Empty\nparameters: {}\n";
        assert!(matches!(
            DevicePack::from_yaml(src),
            Err(PackError::NoParameters)
        ));
    }

    #[test]
    fn registry_resolves_packs_by_stable_name() {
        let peak = DevicePack::from_yaml(NOVATION_PEAK_YAML).unwrap();
        let registry = PackRegistry::with_packs([peak]).unwrap();
        assert_eq!(
            registry.get("novation-peak").unwrap().display_name,
            "Novation Peak"
        );
        assert_eq!(registry.names().collect::<Vec<_>>(), vec!["novation-peak"]);
    }

    #[test]
    fn registry_rejects_duplicate_names() {
        let peak = DevicePack::from_yaml(NOVATION_PEAK_YAML).unwrap();
        let err = PackRegistry::with_packs([peak.clone(), peak]).unwrap_err();
        assert!(matches!(err, PackError::DuplicatePack(name) if name == "novation-peak"));
    }

    #[test]
    fn unknown_pack_error_lists_available_names() {
        let peak = DevicePack::from_yaml(NOVATION_PEAK_YAML).unwrap();
        let registry = PackRegistry::with_packs([peak]).unwrap();
        let err = registry.get("moog").unwrap_err();
        assert!(matches!(
            err,
            PackError::UnknownPack { name, available }
                if name == "moog" && available == vec!["novation-peak"]
        ));
    }
}
