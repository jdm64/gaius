use gaius::skills::{SKILL_NAME_PATTERN, Skill, SkillRepo};
use regex::Regex;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

fn test_skills_dir() -> PathBuf {
    let tid = std::thread::current().id();
    let dir = std::env::temp_dir().join(format!(
        "gaius-skills-test-{}-{:?}",
        std::process::id(),
        tid
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("skills")).unwrap();
    dir
}

fn cleanup_test_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_parse_valid_skill() {
    let dir = test_skills_dir();
    let skill_dir = dir.join("skills").join("my-skill");
    fs::create_dir_all(&skill_dir).unwrap();

    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: my-skill\ndescription: A test skill\n---\n\nSkill body content here.",
    )
    .unwrap();

    let name_regex = Regex::new(SKILL_NAME_PATTERN).unwrap();
    let skill = SkillRepo::parse_skill_file(&skill_dir.join("SKILL.md"), &name_regex).unwrap();

    assert_eq!(skill.name, "my-skill");
    assert_eq!(skill.description, "A test skill");
    assert_eq!(skill.body, "Skill body content here.");

    cleanup_test_dir(&dir);
}

#[test]
fn test_parse_skill_invalid_name() {
    let dir = test_skills_dir();
    let skill_dir = dir.join("skills").join("bad-name");
    fs::create_dir_all(&skill_dir).unwrap();

    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: Bad-Name\ndescription: A test skill\n---\n\nBody.",
    )
    .unwrap();

    let name_regex = Regex::new(SKILL_NAME_PATTERN).unwrap();
    let result = SkillRepo::parse_skill_file(&skill_dir.join("SKILL.md"), &name_regex);

    match &result {
        Ok(skill) => panic!(
            "Expected error but got skill name='{}', description='{}'",
            skill.name, skill.description
        ),
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("Invalid skill name"),
                "Error message doesn't contain 'Invalid skill name': {}",
                msg
            );
        }
    }

    cleanup_test_dir(&dir);
}

#[test]
fn test_parse_skill_missing_fields() {
    let dir = test_skills_dir();
    let skill_dir = dir.join("skills").join("incomplete");
    fs::create_dir_all(&skill_dir).unwrap();

    // Missing description
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: incomplete\n---\n\nBody.",
    )
    .unwrap();

    let name_regex = Regex::new(SKILL_NAME_PATTERN).unwrap();
    let result = SkillRepo::parse_skill_file(&skill_dir.join("SKILL.md"), &name_regex);

    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Missing 'description'")
    );

    cleanup_test_dir(&dir);
}

#[test]
fn test_skill_repo_find() {
    let mut repo = SkillRepo::default();
    repo.skills.insert(
        "test".to_string(),
        Skill {
            name: "test".to_string(),
            description: "A test skill".to_string(),
            body: "Test body".to_string(),
        },
    );

    assert!(repo.find("test").is_ok());
    assert_eq!(repo.find("test").unwrap(), "Test body");
    assert!(repo.find("nonexistent").is_err());
}

#[test]
fn test_skill_repo_sys_prompt() {
    let mut repo = SkillRepo::default();
    repo.skills.insert(
        "alpha".to_string(),
        Skill {
            name: "alpha".to_string(),
            description: "Alpha skill".to_string(),
            body: "Alpha body".to_string(),
        },
    );
    repo.skills.insert(
        "beta".to_string(),
        Skill {
            name: "beta".to_string(),
            description: "Beta skill".to_string(),
            body: "Beta body".to_string(),
        },
    );

    let prompt = repo.sys_prompt().unwrap();
    assert!(prompt.contains("## Skills"));
    assert!(prompt.contains("- alpha: Alpha skill"));
    assert!(prompt.contains("- beta: Beta skill"));

    // Verify sorted order
    let alpha_pos = prompt.find("alpha").unwrap();
    let beta_pos = prompt.find("beta").unwrap();
    assert!(alpha_pos < beta_pos);
}

#[test]
fn test_skill_repo_empty() {
    let repo = SkillRepo::default();
    assert!(repo.is_empty());
    assert_eq!(repo.len(), 0);
    assert!(repo.sys_prompt().is_none());
}

#[test]
fn test_name_validation_patterns() {
    let name_regex = Regex::new(SKILL_NAME_PATTERN).unwrap();

    // Valid names
    assert!(name_regex.is_match("a"));
    assert!(name_regex.is_match("hello"));
    assert!(name_regex.is_match("my-skill"));
    assert!(name_regex.is_match("my-cool-skill"));
    assert!(name_regex.is_match("skill123"));
    assert!(name_regex.is_match("a1b2c3"));
    assert!(name_regex.is_match("123"));
    assert!(name_regex.is_match("a-b"));

    // Invalid names
    assert!(!name_regex.is_match("")); // empty
    assert!(!name_regex.is_match("Hello")); // uppercase
    assert!(!name_regex.is_match("-skill")); // leading hyphen
    assert!(!name_regex.is_match("skill-")); // trailing hyphen
    assert!(!name_regex.is_match("my--skill")); // consecutive hyphens
    assert!(!name_regex.is_match("my_skill")); // underscore
    assert!(!name_regex.is_match("my skill")); // space
    assert!(!name_regex.is_match("my.skill")); // period
}
