//! Integration test: ponytail plugin install under Forge's skill + plugin systems.
//!
//! Verifies that the real upstream `DietrichGebert/ponytail` ruleset (v4.7.0)
//! can be discovered as a Forge skill (markdown + YAML frontmatter) AND loaded
//! as a Forge plugin (forge-plugin.toml manifest) from a fixture directory.
//!
//! Fixtures live in `ext/tests/fixtures/`:
//!   - `skills/ponytail.md`                -> Skill (parsed by `SkillDiscovery`)
//!   - `plugins/ponytail/forge-plugin.toml` -> Plugin (parsed by `PluginLoader` + `PluginRegistry`)
//!
//! If a future refactor breaks either side, this test fails before the rules
//! silently stop loading in user installs.

use std::path::PathBuf;

use ext::plugins::loader::PluginLoader;
use ext::plugins::{PluginRegistry, PluginStatus};
use ext::skills::SkillDiscovery;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn skills_dir() -> PathBuf {
    fixture_root().join("skills")
}

fn plugins_dir() -> PathBuf {
    fixture_root().join("plugins")
}

#[test]
fn ponytail_skill_is_discoverable() {
    let dir = skills_dir();
    assert!(dir.exists(), "skills dir missing: {}", dir.display());
    let discovery = SkillDiscovery::new(dir);
    let skills = discovery.discover().expect("discover should succeed");
    assert_eq!(skills.len(), 1);
    let skill = &skills[0];
    assert_eq!(skill.name, "ponytail");
    assert!(!skill.description.is_empty());
}

#[test]
fn ponytail_skill_carries_the_decision_ladder() {
    let skill = SkillDiscovery::new(skills_dir())
        .get_skill("ponytail")
        .expect("discover should succeed")
        .expect("ponytail skill must be present");
    let prompt = &skill.prompt;
    assert!(prompt.contains("YAGNI"), "missing YAGNI philosophy");
    assert!(prompt.contains("stdlib"), "missing stdlib-first rule");
    assert!(
        prompt.contains("the first rung that holds"),
        "missing ladder framing"
    );
    assert!(
        prompt.contains("ponytail:"),
        "missing ponytail: comment convention"
    );
    assert!(
        prompt.contains("lazy senior"),
        "missing lazy-senior persona"
    );
    assert_eq!(skill.tools.len(), 0, "ponytail declares no tools");
}

#[test]
fn ponytail_plugin_manifest_loads_via_plugin_loader() {
    let plugins =
        PluginLoader::load_from_dir(&plugins_dir()).expect("load_from_dir should succeed");
    assert_eq!(plugins.len(), 1);
    let p = &plugins[0];
    assert_eq!(p.name, "ponytail");
    assert_eq!(p.version.to_string(), "4.7.0");
    assert_eq!(p.provided_prompts, vec!["ponytail".to_string()]);
    assert_eq!(p.provided_tools.len(), 0);
    assert_eq!(p.provided_resources.len(), 0);
}

#[test]
fn ponytail_plugin_registry_loads_with_upstream_metadata() {
    let mut registry = PluginRegistry::new(plugins_dir());
    registry
        .load_from_dir()
        .expect("PluginRegistry::load_from_dir should succeed");
    assert_eq!(registry.count(), 1);
    assert!(registry.is_installed("ponytail"));
    let plugin = registry
        .get("ponytail")
        .expect("ponytail should be installed");
    assert!(matches!(plugin.status, PluginStatus::Installed));
    let meta = &plugin.manifest.metadata;
    assert_eq!(meta.author, "Dietrich Gebert");
    assert_eq!(meta.license.as_deref(), Some("MIT"));
    assert_eq!(
        meta.repository.as_deref(),
        Some("https://github.com/DietrichGebert/ponytail")
    );
    assert_eq!(
        meta.homepage.as_deref(),
        Some("https://github.com/DietrichGebert/ponytail")
    );
    assert!(
        meta.installed_at.starts_with("2026-"),
        "installed_at should be a real ISO timestamp, got {}",
        meta.installed_at
    );
    assert!(plugin.manifest.keywords.contains(&"yagni".to_string()));
    assert!(plugin.manifest.keywords.contains(&"minimalism".to_string()));
}

#[test]
fn ponytail_plugin_search_matches_by_keyword() {
    let mut registry = PluginRegistry::new(plugins_dir());
    registry.load_from_dir().expect("load should succeed");
    let results = registry.search("yagni");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].manifest.name, "ponytail");
}

#[test]
fn ponytail_skill_and_plugin_agree_on_name() {
    // The skill (frontmatter name) and the plugin (manifest name) must
    // agree, otherwise downstream code that pairs them by name breaks.
    let skill = SkillDiscovery::new(skills_dir())
        .get_skill("ponytail")
        .expect("discover ok")
        .expect("ponytail skill present");
    let skill_name = skill.name.clone();
    let mut registry = PluginRegistry::new(plugins_dir());
    registry.load_from_dir().expect("load ok");
    let plugin = registry
        .get(&skill_name)
        .unwrap_or_else(|| panic!("no plugin matching skill name {skill_name}"));
    assert_eq!(plugin.manifest.name, skill_name);
}
