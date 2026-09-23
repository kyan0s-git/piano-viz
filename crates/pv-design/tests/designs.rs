use pv_design::*;

#[test]
fn every_builtin_loads_without_warnings() {
    for (id, src) in BUILTIN {
        let loaded = from_toml(src).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(loaded.warnings.is_empty(), "{id}: {:?}", loaded.warnings);
    }
}

#[test]
fn ember_classic_file_matches_code_default() {
    let mut file = builtin("ember-classic");
    file.meta = Meta::default();
    assert_eq!(file, Design::default());
}

#[test]
fn minimal_and_print_really_have_no_particles() {
    assert!(builtin("minimal").particles.is_empty());
    assert!(builtin("print").particles.is_empty());
}

#[test]
fn default_round_trips_through_toml() {
    let d = Design::default();
    assert_eq!(from_toml(&to_toml(&d)).unwrap().design, d);
}

#[test]
fn sparse_design_fills_in_defaults() {
    let l = from_toml("[notes]\ncorner_radius = 9.0\n").unwrap();
    assert_eq!(l.design.notes.corner_radius, 9.0);
    assert_eq!(l.design.notes.border_width, NoteStyle::default().border_width);
    assert!(l.warnings.is_empty());
}

#[test]
fn unknown_keys_warn_with_their_path() {
    let src =
        "future_thing = 1\n[notes]\nsparkle = true\n[[particles]]\n[[particles]]\nwobble = 2\n";
    let w = from_toml(src).unwrap().warnings;
    for key in ["`future_thing`", "`notes.sparkle`", "`particles[1].wobble`"] {
        assert!(w.iter().any(|m| m.contains(key)), "missing {key} in {w:?}");
    }
}

#[test]
fn wrong_type_is_an_error_naming_the_line() {
    let err =
        from_toml("[meta]\nname = \"x\"\n\n[notes]\nopacity = \"lots\"\n").unwrap_err().to_string();
    assert!(err.contains("line 5") || err.contains("5 |"), "{err}");
}

#[test]
fn bad_color_is_explained() {
    let err = from_toml("[background]\ncolor = \"red\"\n").unwrap_err().to_string();
    assert!(err.contains("#rrggbb"), "{err}");
}

#[test]
fn out_of_range_values_are_clamped_with_a_warning() {
    let l = from_toml("[layout]\nlookahead = -3.0\n[[particles]]\ncount = 100000\n").unwrap();
    assert_eq!(l.design.layout.lookahead, 0.2);
    assert_eq!(l.design.particles[0].count, MAX_BURST);
    assert_eq!(l.warnings.len(), 2, "{:?}", l.warnings);
}

#[test]
fn newer_schema_warns_but_loads() {
    let l = from_toml("[meta]\nversion = 99\n").unwrap();
    assert!(l.warnings.iter().any(|w| w.contains("newer version")));
}

#[test]
fn key_ranges_parse() {
    let r = |s: &str| {
        from_toml(&format!("[keyboard]\nrange = \"{s}\"\n")).map(|l| l.design.keyboard.range)
    };
    assert_eq!(r("auto").unwrap(), KeyRange::Auto);
    assert_eq!(r("88").unwrap(), KeyRange::Fixed(21, 108));
    assert_eq!(r("36-96").unwrap(), KeyRange::Fixed(36, 96));
    assert!(r("96-36").is_err());
    assert!(r("0-200").is_err());
}

#[test]
fn asset_paths_cannot_escape_the_bundle() {
    let dir = std::env::temp_dir();
    for bad in ["../secret.png", "/etc/passwd", "a/../../b.png", ""] {
        assert!(resolve_asset(&dir, bad).is_err(), "{bad:?} should be rejected");
    }
    assert!(resolve_asset(&dir, "assets/bg.png").is_ok());
    assert!(resolve_asset(&dir, "./bg.png").is_ok());
}

#[test]
fn loads_a_bundle_directory() {
    let dir = std::env::temp_dir().join(format!("pv-design-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("design.toml"), "[meta]\nname = \"Bundle\"\n").unwrap();
    let l = load(&dir).unwrap();
    assert_eq!(l.design.meta.name, "Bundle");
    assert_eq!(l.bundle_dir.as_deref(), Some(dir.as_path()));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn hold_emitter_bounds_alive_count() {
    let e = Emitter::hold_default();
    assert_eq!(e.max_alive_per_note(), (e.rate * e.lifetime).ceil() as u32 + 1);
}
