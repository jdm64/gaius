use gaius::agents::Agents;

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
