use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

pub struct Vocabulary {
    pub relationship_phases: BTreeSet<String>,
    pub comm_styles: BTreeSet<String>,
    pub cadence_modes: BTreeSet<String>,
    pub required_beats: BTreeSet<String>,
    pub forbidden_beats: BTreeSet<String>,
    pub tone_labels: BTreeSet<String>,
    pub authorities: BTreeSet<String>,
    pub visibility_scopes: BTreeSet<String>,
}

impl Vocabulary {
    pub fn load(path: &Path) -> Self {
        let raw = fs::read_to_string(path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        let relationship_phases = labels_in_section(&raw, "Relationship Phases");
        let comm_styles = labels_in_section(&raw, "Communication Style Values");
        let cadence_modes = labels_in_section(&raw, "Cadence Modes");
        let required_beats = labels_in_section(&raw, "Required Beat Labels");
        let forbidden_beats = labels_in_section(&raw, "Forbidden Beat Labels");
        let tone_labels = labels_in_section(&raw, "Tone Labels");

        for (name, values) in [
            ("relationship phases", &relationship_phases),
            ("communication styles", &comm_styles),
            ("cadence modes", &cadence_modes),
            ("required beats", &required_beats),
            ("forbidden beats", &forbidden_beats),
            ("tone labels", &tone_labels),
        ] {
            assert!(
                !values.is_empty(),
                "vocabulary section {name} must not be empty"
            );
        }

        Self {
            relationship_phases,
            comm_styles,
            cadence_modes,
            required_beats,
            forbidden_beats,
            tone_labels,
            authorities: [
                "chosen_person",
                "trusted",
                "default",
                "restricted",
                "blocked",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            visibility_scopes: [
                "profile",
                "person",
                "chosen_person_only",
                "global",
                "public",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }
}

fn labels_in_section(markdown: &str, section: &str) -> BTreeSet<String> {
    let heading = format!("## {section}");
    let mut in_section = false;
    let mut labels = BTreeSet::new();
    for line in markdown.lines() {
        if line == heading {
            in_section = true;
            continue;
        }
        if in_section && line.starts_with("## ") {
            break;
        }
        if in_section {
            if let Some(label) = line.strip_prefix('`').and_then(|rest| rest.split_once('`')) {
                labels.insert(label.0.to_string());
            }
        }
    }
    labels
}
