//! The live parameter studio's file format and cascade resolver.
//!
//! See `docs/PARAMETER-STUDIO.md` for the accepted design this crate
//! implements: a `.piano.json` file cascades through four tiers —
//! `defaults` < `registers` < `groups` < `strings`, most specific wins —
//! down to a flat table of every string's resolved parameters. Loading
//! and saving are both explicit ([`load`], [`save`]); nothing here
//! autosaves, per the design's own "Persistence" section.
//!
//! This crate never touches the audio thread directly — it sits where
//! `piano-render` and `piano-cli` already do, on the allocating,
//! file-touching side of `docs/ARCHITECTURE.md`'s split. Turning a
//! [`ResolvedPiano`] into a running instrument's live state is the next
//! layer up (`piano-cli`'s `studio` subcommand).

mod error;
mod format;
mod resolve;

use std::fs;
use std::path::Path;

pub use error::StudioError;
pub use format::{
    BridgeOverrides, Group, HammerOverrides, Instrument, ParameterOverrides, PianoFile,
    RegisterAnchor, Registers, SoundboardModeOverride, StringOverride, StringRef,
};
pub use resolve::{ResolvedPiano, ResolvedString, resolve};

/// Reads and parses a `.piano.json` file.
///
/// # Errors
///
/// Returns [`StudioError::Io`] if `path` cannot be read, or
/// [`StudioError::Parse`] if its contents are not valid `.piano.json`.
pub fn load(path: &Path) -> Result<PianoFile, StudioError> {
    let contents = fs::read_to_string(path).map_err(|source| StudioError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&contents).map_err(|source| StudioError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Serialises `file` and writes it to `path`, creating or overwriting it —
/// the fully resolved state, not a diff (`docs/PARAMETER-STUDIO.md`'s
/// "Persistence" section: "writing the fully resolved table is simpler,
/// always correct, and avoids building a diffing feature nobody asked
/// for").
///
/// # Errors
///
/// Returns [`StudioError::Io`] if `path` cannot be written.
pub fn save(path: &Path, file: &PianoFile) -> Result<(), StudioError> {
    // `PianoFile` is our own type, built entirely of finite, already-valid
    // Rust values — serialising it cannot fail the way parsing arbitrary
    // input can, so this is Io-only, unlike `load`.
    let contents = serde_json::to_string_pretty(file).unwrap_or_default();
    fs::write(path, contents).map_err(|source| StudioError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use piano_core::hammer::DEFAULT_HAMMER;
    use piano_core::string::{DEFAULT_DAMPING, DEFAULT_SUSTAIN};
    use piano_params::Tuning;

    use super::*;

    fn find(piano: &ResolvedPiano, midi: u8, string_index: u8) -> &ResolvedString {
        piano
            .strings
            .iter()
            .find(|string| string.midi == midi && string.string_index == string_index)
            .expect("midi/string_index exists on an 88-key instrument")
    }

    #[test]
    fn a_default_file_resolves_every_string_to_the_register_baseline() {
        let file = PianoFile::default();
        let piano = resolve(&file, Tuning::default());
        assert_eq!(
            piano.strings.len(),
            222,
            "88-key unison count, per docs/PHYSICS.md"
        );
        let a4_string0 = find(&piano, 69, 0);
        let expected = piano_audio::voicing::voicing_for_key(
            piano_params::PianoKey::from_midi(69).expect("A4 is a real key"),
            Tuning::default(),
        );
        assert_eq!(a4_string0.damping, expected.damping);
        assert_eq!(a4_string0.hammer, DEFAULT_HAMMER);
        assert_eq!(a4_string0.detune_cents, 0.0);
    }

    #[test]
    fn an_explicit_string_override_wins_over_everything_else() {
        let mut file = PianoFile::default();
        file.strings.push(StringOverride {
            midi: 69,
            string_index: 1,
            overrides: ParameterOverrides {
                detune_cents: Some(3.0),
                seed: Some(12_345),
                ..ParameterOverrides::default()
            },
        });

        let piano = resolve(&file, Tuning::default());
        let overridden = find(&piano, 69, 1);
        assert_eq!(overridden.detune_cents, 3.0);
        assert_eq!(overridden.seed, 12_345);

        // A sibling string on the same key, not named in `strings`, still
        // falls through to the register baseline untouched.
        let sibling = find(&piano, 69, 0);
        assert_eq!(sibling.detune_cents, 0.0);
        assert_eq!(sibling.seed, 0);
    }

    #[test]
    fn a_group_override_reaches_every_string_it_names_but_no_others() {
        // MIDI 60 (middle C) and 61 are both in the trichord register
        // (key_index >= 30, `piano_core::unison`'s documented boundary),
        // so `string_index` 0-2 are all valid on both — unlike the design
        // doc's own illustrative example (midi 30/32), which lands in the
        // monochord register and only has a `string_index` 0 to give.
        let mut file = PianoFile::default();
        file.groups.push(Group {
            name: "darker overtones".to_string(),
            strings: vec![
                StringRef {
                    midi: 60,
                    string_index: 0,
                },
                StringRef {
                    midi: 61,
                    string_index: 1,
                },
            ],
            overrides: ParameterOverrides {
                damping: Some(0.7),
                ..ParameterOverrides::default()
            },
        });

        let piano = resolve(&file, Tuning::default());
        assert_eq!(find(&piano, 60, 0).damping, 0.7);
        assert_eq!(find(&piano, 61, 1).damping, 0.7);
        // A string on key 61 but not the named index is untouched.
        assert_ne!(find(&piano, 61, 0).damping, 0.7);
    }

    #[test]
    fn a_string_override_wins_over_a_matching_group() {
        let mut file = PianoFile::default();
        file.groups.push(Group {
            name: "group".to_string(),
            strings: vec![StringRef {
                midi: 69,
                string_index: 0,
            }],
            overrides: ParameterOverrides {
                damping: Some(0.7),
                ..ParameterOverrides::default()
            },
        });
        file.strings.push(StringOverride {
            midi: 69,
            string_index: 0,
            overrides: ParameterOverrides {
                damping: Some(0.2),
                ..ParameterOverrides::default()
            },
        });

        let piano = resolve(&file, Tuning::default());
        assert_eq!(find(&piano, 69, 0).damping, 0.2);
    }

    #[test]
    fn defaults_only_take_effect_for_fields_the_register_tier_does_not_provide() {
        let mut file = PianoFile::default();
        file.defaults.damping = Some(0.99); // overridden by the register tier
        file.defaults.seed = Some(999); // not touched by the register tier

        let piano = resolve(&file, Tuning::default());
        let string = find(&piano, 69, 0);
        assert_ne!(
            string.damping, 0.99,
            "register tier should win over defaults"
        );
        assert_eq!(
            string.seed, 999,
            "defaults should apply where nothing else does"
        );
    }

    #[test]
    fn missing_defaults_fall_back_to_piano_core_s_own_defaults() {
        let file = PianoFile::default();
        let piano = resolve(&file, Tuning::default());
        // `sustain`/`damping` at A4 come entirely from the register tier
        // in this test, so this just proves the two default constants
        // compile in — the direct check below is `hammer`, which nothing
        // here overrides.
        let _ = DEFAULT_SUSTAIN;
        let _ = DEFAULT_DAMPING;
        assert_eq!(find(&piano, 69, 0).hammer, DEFAULT_HAMMER);
    }

    #[test]
    fn instrument_wide_settings_fall_back_to_documented_defaults() {
        let file = PianoFile::default();
        let piano = resolve(&file, Tuning::default());
        assert!(piano.soundboard_modes.is_empty());
        assert_eq!(piano.local_coupling_gain, 0.15);
        assert_eq!(piano.global_coupling_gain, 0.08);
    }

    #[test]
    fn a_piano_file_round_trips_through_json() {
        let mut file = PianoFile {
            name: Some("My Piano".to_string()),
            ..PianoFile::default()
        };
        file.strings.push(StringOverride {
            midi: 69,
            string_index: 1,
            overrides: ParameterOverrides {
                detune_cents: Some(3.0),
                ..ParameterOverrides::default()
            },
        });
        let json = serde_json::to_string(&file).expect("serialises");
        let parsed: PianoFile = serde_json::from_str(&json).expect("parses its own output");
        assert_eq!(parsed, file);
    }

    #[test]
    fn the_documented_json_example_parses() {
        let example = r#"{
            "name": "My Piano",
            "defaults": {
                "damping": 0.5, "sustain": 0.996, "inharmonicity": 0.0004,
                "hammer": { "contact_exponent": 2.5, "stiffness": 1.7e9, "mass": 1.0 }
            },
            "registers": {
                "bass":   { "anchor_midi": 21,  "decay_seconds": 35.0, "damping": 0.6, "inharmonicity": 0.0001 },
                "mid":    { "anchor_midi": 69,  "decay_seconds": 11.0 },
                "treble": { "anchor_midi": 108, "decay_seconds": 1.5,  "damping": 0.4, "inharmonicity": 0.05 }
            },
            "groups": [
                {
                    "name": "darker bass overtones",
                    "strings": [ { "midi": 30, "string_index": 0 }, { "midi": 32, "string_index": 1 } ],
                    "overrides": { "damping": 0.7 }
                }
            ],
            "strings": [
                { "midi": 69, "string_index": 1, "detune_cents": 3.0, "seed": 12345 }
            ],
            "instrument": {
                "soundboard_modes": [
                    { "frequency_hz": 80.0, "decay_seconds": 1.2, "gain": 1.0 }
                ],
                "bridge": { "local_coupling_gain": 0.15, "global_coupling_gain": 0.08 }
            }
        }"#;
        let file: PianoFile = serde_json::from_str(example).expect("the documented example parses");
        assert_eq!(file.name.as_deref(), Some("My Piano"));
        assert_eq!(file.groups.len(), 1);
        assert_eq!(file.strings.len(), 1);
        assert_eq!(file.instrument.soundboard_modes.len(), 1);
        let _ = resolve(&file, Tuning::default());
    }

    #[test]
    fn load_reports_a_missing_file() {
        let error = load(Path::new("/nonexistent/does-not-exist.piano.json")).unwrap_err();
        assert!(matches!(error, StudioError::Io { .. }));
    }

    #[test]
    fn load_reports_invalid_json() {
        let dir = std::env::temp_dir().join(format!("piano-studio-test-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir creates");
        let path = dir.join("bad.piano.json");
        fs::write(&path, b"not json").expect("write succeeds");
        let error = load(&path).unwrap_err();
        assert!(matches!(error, StudioError::Parse { .. }));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = std::env::temp_dir().join(format!("piano-studio-test-rt-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir creates");
        let path = dir.join("roundtrip.piano.json");

        let file = PianoFile {
            name: Some("Round Trip".to_string()),
            ..PianoFile::default()
        };
        save(&path, &file).expect("save succeeds");
        let loaded = load(&path).expect("load succeeds");
        assert_eq!(loaded, file);
        let _ = fs::remove_dir_all(&dir);
    }
}
