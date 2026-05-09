/* Copyright 2026 Justin Madru (justin.jdm64@gmail.com)
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use std::{error::Error, path::Path};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentDefinition {
    pub name: String,
    pub prompt: String,
}

#[derive(Debug, Clone, Default)]
pub struct Agents {
    agents: Vec<AgentDefinition>,
}

impl Agents {
    pub fn load(config_dir: &Path) -> Result<Self, Box<dyn Error>> {
        let mut agents = hardcoded_agents();
        let agents_dir = config_dir.join("agents");

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
}

fn hardcoded_agents() -> Vec<AgentDefinition> {
    vec![AgentDefinition {
        name: "basic".to_string(),
        prompt: String::new(),
    }]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_basic_agent_by_default() {
        let temp_dir = tempfile_dir();
        let agents = Agents::load(&temp_dir).unwrap();

        assert_eq!(agents.default_agent().name, "basic");
        assert_eq!(agents.default_agent().prompt, "");
    }

    #[test]
    fn loads_user_defined_agents_from_toml_files() {
        let temp_dir = tempfile_dir();
        let agents_dir = temp_dir.join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        std::fs::write(
            agents_dir.join("planner.toml"),
            r#"name = "planner"
prompt = "Plan before answering."
"#,
        )
        .unwrap();

        let agents = Agents::load(&temp_dir).unwrap();
        let planner = agents.find("planner").unwrap();

        assert_eq!(planner.prompt, "Plan before answering.");
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("gaius-test-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}
