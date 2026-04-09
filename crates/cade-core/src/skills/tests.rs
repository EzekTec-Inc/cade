#[allow(unused)]
type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

use super::*;
use std::fs;
use std::path::{Path, PathBuf};

// -- SkillScope ordering

#[test]
fn scope_ordering() {
    assert!(SkillScope::Project > SkillScope::Agent);
    assert!(SkillScope::Agent > SkillScope::Global);
    assert!(SkillScope::Global > SkillScope::Builtin);
}

#[test]
fn scope_display() {
    assert_eq!(SkillScope::Builtin.to_string(), "builtin");
    assert_eq!(SkillScope::Global.to_string(), "global");
    assert_eq!(SkillScope::Agent.to_string(), "agent");
    assert_eq!(SkillScope::Project.to_string(), "project");
}

// -- Frontmatter parsing

#[test]
fn parse_skill_minimal() -> Result<()> {
    let content = "---\nname: Test Skill\ndescription: A test\n---\nBody here.";
    let skill = parse_skill(
        "test-skill",
        content,
        SkillScope::Project,
        PathBuf::from("/fake/SKILL.MD"),
    )?;
    assert_eq!(skill.id, "test-skill");
    assert_eq!(skill.name, "Test Skill");
    assert_eq!(skill.description, "A test");
    assert_eq!(skill.body, "Body here.");
    assert_eq!(skill.scope, SkillScope::Project);

    Ok(())
}

#[test]
fn parse_skill_with_tags_inline() -> Result<()> {
    let content = "---\nname: S\ndescription: D\ntags: [\"rust\", \"testing\"]\n---\nBody";
    let skill = parse_skill("s", content, SkillScope::Global, PathBuf::from("/f"))?;
    assert_eq!(skill.tags, vec!["rust", "testing"]);

    Ok(())
}

#[test]
fn parse_skill_with_tags_multiline() -> Result<()> {
    let content = "---\nname: S\ndescription: D\ntags:\n  - rust\n  - testing\n---\nBody";
    let skill = parse_skill("s", content, SkillScope::Global, PathBuf::from("/f"))?;
    assert_eq!(skill.tags, vec!["rust", "testing"]);

    Ok(())
}

#[test]
fn parse_skill_with_triggers() -> Result<()> {
    let content = "---\nname: S\ndescription: D\ntriggers: [debug, \"fix error\"]\n---\nBody";
    let skill = parse_skill("s", content, SkillScope::Global, PathBuf::from("/f"))?;
    assert_eq!(skill.triggers, vec!["debug", "fix error"]);

    Ok(())
}

#[test]
fn parse_skill_with_tools_block() -> Result<()> {
    let content = "---\nname: S\ndescription: D\ntools:\n  - name: my_tool\n    description: does stuff\n    entrypoint: scripts/my_tool.sh\n---\nBody";
    let skill = parse_skill("s", content, SkillScope::Global, PathBuf::from("/f"))?;
    // tools are parsed but scripts require actual disk files — verify frontmatter parsed
    assert_eq!(skill.body, "Body");

    Ok(())
}

#[test]
fn parse_skill_no_frontmatter() -> Result<()> {
    let content = "Just a body with no frontmatter.";
    let skill = parse_skill("bare", content, SkillScope::Builtin, PathBuf::from("/f"))?;
    assert_eq!(skill.name, "bare"); // falls back to id
    assert_eq!(skill.description, "");
    assert_eq!(skill.body, "Just a body with no frontmatter.");

    Ok(())
}

#[test]
fn parse_skill_rpi_phase() -> Result<()> {
    let content = "---\nname: S\ndescription: D\nrpi_phase: Implement\n---\nBody";
    let skill = parse_skill("s", content, SkillScope::Global, PathBuf::from("/f"))?;
    assert_eq!(skill.rpi_phase.as_deref(), Some("Implement"));

    Ok(())
}

// -- Skill::matches_trigger

fn make_skill(triggers: Vec<&str>) -> Skill {
    Skill {
        id: "test".into(),
        name: "Test".into(),
        description: "".into(),
        category: None,
        tags: vec![],
        triggers: triggers.into_iter().map(String::from).collect(),
        rpi_phase: None,
        capabilities: vec![],
        scripts: vec![],
        references: vec![],
        body: "".into(),
        scope: SkillScope::Project,
        path: PathBuf::from("/f"),
    }
}

#[test]
fn trigger_single_word_exact() {
    let s = make_skill(vec!["debug"]);
    assert!(s.matches_trigger("Help me debug this error"));
    assert!(s.matches_trigger("debug"));
    assert!(!s.matches_trigger("debugging")); // not a word boundary match
}

#[test]
fn trigger_multi_word_substring() -> Result<()> {
    let s = make_skill(vec!["fix error"]);
    assert!(s.matches_trigger("Can you fix error in module?"));
    assert!(!s.matches_trigger("fix the error")); // not exact substring

    Ok(())
}

#[test]
fn trigger_case_insensitive() {
    let s = make_skill(vec!["Debug"]);
    assert!(s.matches_trigger("DEBUG this please"));
    assert!(s.matches_trigger("debug this please"));
}

#[test]
fn trigger_empty_triggers() {
    let s = make_skill(vec![]);
    assert!(!s.matches_trigger("anything"));
}

// -- Skill::listing_line

#[test]
fn listing_line_format() {
    let mut s = make_skill(vec![]);
    s.category = Some("code".into());
    s.rpi_phase = Some("Implement".into());
    s.description = "A useful skill".into();
    let line = s.listing_line();
    assert!(line.contains("test"));
    assert!(line.contains("[code]"));
    assert!(line.contains("<Implement>"));
    assert!(line.contains("A useful skill"));
}

// -- skills_listing

#[test]
fn skills_listing_empty() {
    assert!(skills_listing(&[]).is_none());
}

#[test]
fn skills_listing_nonempty() -> Result<()> {
    let s = make_skill(vec![]);
    let listing = skills_listing(&[s]).ok_or("Should produce listing")?;
    assert!(listing.contains("Available Skills"));

    Ok(())
}

// -- github_url_to_raw_skill

#[test]
fn github_tree_url_conversion() -> Result<()> {
    let url = "https://github.com/user/repo/tree/main/skills/my-skill";
    let raw = github_url_to_raw_skill(url).ok_or("Should convert URL")?;
    assert_eq!(
        raw,
        "https://raw.githubusercontent.com/user/repo/main/skills/my-skill/SKILL.MD"
    );

    Ok(())
}

#[test]
fn github_blob_url_conversion() -> Result<()> {
    let url = "https://github.com/user/repo/blob/main/skills/SKILL.MD";
    let raw = github_url_to_raw_skill(url).ok_or("Should convert URL")?;
    assert!(raw.starts_with("https://raw.githubusercontent.com/"));

    Ok(())
}

#[test]
fn non_github_url_returns_none() {
    assert!(github_url_to_raw_skill("https://example.com/skills").is_none());
}

// -- discover_skills_in (filesystem)

#[test]
fn discover_skills_empty_dir() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let skills = discover_skills_in(dir.path(), SkillScope::Project);
    assert!(skills.is_empty());

    Ok(())
}

#[test]
fn discover_skills_finds_skill_md() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let skill_dir = dir.path().join("my-skill");
    fs::create_dir_all(&skill_dir)?;
    fs::write(
        skill_dir.join("SKILL.MD"),
        "---\nname: My Skill\ndescription: Test\n---\nBody",
    )?;

    let skills = discover_skills_in(dir.path(), SkillScope::Project);
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].id, "my-skill");
    assert_eq!(skills[0].name, "My Skill");
    assert_eq!(skills[0].scope, SkillScope::Project);

    Ok(())
}

#[test]
fn discover_skills_nonexistent_dir() {
    let skills = discover_skills_in(Path::new("/nonexistent/path"), SkillScope::Global);
    assert!(skills.is_empty());
}

// -- discover_all_skills merging

#[test]
fn discover_all_skills_higher_scope_wins() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cade_home = tempfile::tempdir()?;

    // Global skill
    let global_dir = cade_home.path().join("skills").join("shared");
    fs::create_dir_all(&global_dir)?;
    fs::write(
        global_dir.join("SKILL.MD"),
        "---\nname: Global Version\ndescription: global\n---\nGlobal body",
    )?;

    // Project skill with same ID
    let proj_dir = dir.path().join(".cade/skills").join("shared");
    fs::create_dir_all(&proj_dir)?;
    fs::write(
        proj_dir.join("SKILL.MD"),
        "---\nname: Project Version\ndescription: project\n---\nProject body",
    )?;

    let skills = discover_all_skills(dir.path(), None, Some(cade_home.path()));
    let shared = skills
        .iter()
        .find(|s| s.id == "shared")
        .ok_or("Should find skill")?;
    assert_eq!(shared.name, "Project Version"); // project scope wins
    assert_eq!(shared.scope, SkillScope::Project);

    Ok(())
}
