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

use serde::{Deserialize, Serialize};
use std::{error::Error, path::Path};

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
