/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use regex::Regex;
use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
}

#[derive(Debug, Clone, Default)]
pub struct SkillRepo {
    pub skills: HashMap<String, Skill>,
}

pub const SKILL_NAME_PATTERN: &str = r"^[a-z0-9]+(-[a-z0-9]+)*$";
const SKILL_NAME_MAX_LEN: usize = 64;
const DESCRIPTION_MAX_LEN: usize = 1024;

impl SkillRepo {
    pub fn load() -> Result<Self, Box<dyn Error>> {
        let config_dir = crate::dirs::Dirs::config_dir()?;
        let skills_dir = config_dir.join("skills");
        let mut repo = Self::default();

        if !skills_dir.exists() {
            return Ok(repo);
        }

        let name_regex = Regex::new(SKILL_NAME_PATTERN)?;

        let entries = match std::fs::read_dir(&skills_dir) {
            Ok(entries) => entries,
            Err(e) => {
                eprintln!("Warning: Failed to read skills directory: {}", e);
                return Ok(repo);
            }
        };

        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let dir_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

            // Check for SKILL.md file
            let skill_file = path.join("SKILL.md");
            if !skill_file.exists() {
                eprintln!(
                    "Warning: Directory '{}' in skills/ does not contain SKILL.md, skipping",
                    dir_name
                );
                continue;
            }

            // Parse the SKILL.md file
            match Self::parse_skill_file(&skill_file, &name_regex) {
                Ok(skill) => {
                    // Validate directory name matches skill name
                    if skill.name != dir_name {
                        eprintln!(
                            "Warning: Skill name '{}' does not match directory name '{}', skipping",
                            skill.name, dir_name
                        );
                        continue;
                    }
                    repo.skills.insert(skill.name.clone(), skill);
                }
                Err(e) => {
                    eprintln!("Warning: Failed to parse skill in '{}': {}", dir_name, e);
                    continue;
                }
            }
        }

        Ok(repo)
    }

    pub fn find(&self, name: &str) -> Result<String, Box<dyn Error>> {
        self.skills
            .get(name)
            .map(|skill| skill.body.clone())
            .ok_or_else(|| format!("Skill '{}' not found", name).into())
    }

    pub fn sys_prompt(&self) -> Option<String> {
        if self.skills.is_empty() {
            return None;
        }

        let mut lines = vec!["## Skills".to_string()];

        let mut sorted_skills: Vec<&Skill> = self.skills.values().collect();
        sorted_skills.sort_by_key(|s| &s.name);

        for skill in sorted_skills {
            lines.push(format!("- {}: {}", skill.name, skill.description));
        }

        Some(lines.join("\n"))
    }

    /// Parse a SKILL.md file with YAML frontmatter.
    ///
    /// Format:
    /// ```markdown
    /// ---
    /// name: my-skill
    /// description: Description of what this skill does
    /// ---
    ///
    /// Body of the skill containing instructions, examples, etc.
    /// ```
    pub fn parse_skill_file(path: &Path, name_regex: &Regex) -> Result<Skill, Box<dyn Error>> {
        let content = std::fs::read_to_string(path)?;
        let trimmed = content.trim();

        if !trimmed.starts_with("---") {
            return Err("Missing YAML frontmatter (file must start with ---)".into());
        }

        let rest = &trimmed[3..];
        let end_idx = rest.find("\n---").ok_or("Unclosed YAML frontmatter")?;
        let frontmatter = rest[..end_idx].trim();
        let body = rest[end_idx + 4..].trim().to_string(); // skip "\n---" (4 chars)

        let mut name = None;
        let mut description = None;

        for line in frontmatter.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let (key, value) = match line.split_once(':') {
                Some((k, v)) => (k.trim(), v.trim()),
                None => continue,
            };

            match key {
                "name" => name = Some(value.to_string()),
                "description" => description = Some(value.to_string()),
                _ => {} // Ignore unknown fields
            }
        }

        let name = name.ok_or("Missing 'name' field in frontmatter")?;
        let description = description.ok_or("Missing 'description' field in frontmatter")?;

        if name.is_empty() || name.len() > SKILL_NAME_MAX_LEN {
            return Err(format!(
                "Skill name must be 1-{} characters, got '{}' (len={})",
                SKILL_NAME_MAX_LEN,
                name,
                name.len()
            )
            .into());
        }

        if !name_regex.is_match(&name) {
            return Err(format!(
                "Invalid skill name '{}': must be lowercase alphanumeric with single hyphen separators",
                name
            )
            .into());
        }

        if description.is_empty() || description.len() > DESCRIPTION_MAX_LEN {
            return Err(format!(
                "Description must be 1-{} characters, got '{}' (len={})",
                DESCRIPTION_MAX_LEN,
                description,
                description.len()
            )
            .into());
        }

        Ok(Skill {
            name,
            description,
            body,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    pub fn len(&self) -> usize {
        self.skills.len()
    }

    pub fn insert(&mut self, skill: Skill) {
        self.skills.insert(skill.name.clone(), skill);
    }

    pub fn list(&self) -> Vec<Skill> {
        let mut skills: Vec<Skill> = self.skills.values().cloned().collect();
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        skills
    }
}
