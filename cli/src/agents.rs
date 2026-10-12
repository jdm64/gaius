/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::tools::ToolName;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{error::Error, path::Path};

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentDefinition {
    pub name: String,
    pub prompt: String,
    #[serde(default)]
    pub tools: AgentTools,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentTools {
    #[serde(
        default,
        serialize_with = "serialize_vec",
        deserialize_with = "deserialize_vec"
    )]
    pub include: Vec<String>,
    #[serde(
        default,
        serialize_with = "serialize_vec",
        deserialize_with = "deserialize_vec"
    )]
    pub exclude: Vec<String>,
}

impl AgentTools {
    pub fn has_filters(&self) -> bool {
        !self.include.is_empty() || !self.exclude.is_empty()
    }
}

fn serialize_vec<S>(value: &[String], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.join(","))
}

fn deserialize_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(if s.is_empty() {
        Vec::new()
    } else {
        s.split(',').map(String::from).collect()
    })
}

#[derive(Debug, Clone, Default)]
pub struct Agents {
    agents: Vec<AgentDefinition>,
}

impl Agents {
    pub fn load(config_dir: &Path) -> Result<Self, Box<dyn Error>> {
        let mut agents = hardcoded_agents();
        let agents_dir = config_dir.join("agents");
        std::fs::create_dir_all(&agents_dir)?;

        if agents_dir.is_dir() {
            let mut files = std::fs::read_dir(&agents_dir)?
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
                .collect::<Vec<_>>();
            files.sort();

            for path in files {
                let contents = std::fs::read_to_string(&path)?;
                let agent: AgentDefinition = toml::from_str(&contents)
                    .map_err(|err| format!("Failed to load agent {}: {}", path.display(), err))?;
                agents.push(agent);
            }
        }

        validate_agents(&agents)?;
        Ok(Self { agents })
    }

    pub fn all(&self) -> &[AgentDefinition] {
        &self.agents
    }

    pub fn default_agent(&self) -> &AgentDefinition {
        self.find("basic")
            .expect("hardcoded agents must include a basic agent")
    }

    pub fn find(&self, name: &str) -> Option<&AgentDefinition> {
        self.agents.iter().find(|agent| agent.name == name)
    }

    pub fn next_agent(&self, name: &str) -> Option<&AgentDefinition> {
        let current_index = self.agents.iter().position(|agent| agent.name == name)?;
        let next_index = (current_index + 1) % self.agents.len();
        self.agents.get(next_index)
    }

    pub fn mark_recent(&mut self, name: &str) {
        if let Some(current_index) = self.agents.iter().position(|agent| agent.name == name)
            && current_index != 0
        {
            let agent = self.agents.remove(current_index);
            self.agents.insert(0, agent);
        }
    }
}

fn hardcoded_agents() -> Vec<AgentDefinition> {
    vec![
        AgentDefinition {
            name: "basic".to_string(),
            prompt: String::new(),
            tools: AgentTools::default(),
        },
        AgentDefinition {
            name: "minimal".to_string(),
            prompt: String::new(),
            tools: AgentTools {
                include: vec![
                    ToolName::ReadFile.name(),
                    ToolName::CreateFile.name(),
                    ToolName::EditFile.name(),
                    ToolName::Bash.name(),
                ],
                exclude: Vec::new(),
            },
        },
    ]
}

fn validate_agents(agents: &[AgentDefinition]) -> Result<(), Box<dyn Error>> {
    for agent in agents {
        if agent.name.trim().is_empty() {
            return Err("Agent name cannot be empty".into());
        }
    }

    for (index, agent) in agents.iter().enumerate() {
        if agents
            .iter()
            .skip(index + 1)
            .any(|other| other.name == agent.name)
        {
            return Err(format!("Duplicate agent name: {}", agent.name).into());
        }
    }

    Ok(())
}
