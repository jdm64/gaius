use gaius::{
    actor::{CommandReply, HarnessActorClient, HarnessCommand, HarnessRxHandle},
    commands::Command,
    config::Config,
    harness::{Harness, HarnessSnapshot},
    input::InputMode,
    skills::Skill,
    token_usage::SessionInfo,
    tui::TuiApp,
};

/// An actor with the snapshot it started from.
fn actor() -> (HarnessActorClient, HarnessRxHandle, HarnessSnapshot) {
    let harness = Harness::new(&Config::default(), None).unwrap();
    let snapshot = harness.snapshot();
    let (actor, rx) = HarnessActorClient::new(harness).unwrap();
    (actor, rx, snapshot)
}

fn skill(name: &str) -> Skill {
    Skill {
        name: name.to_string(),
        description: format!("the {name} skill"),
        body: String::new(),
    }
}

#[tokio::test]
async fn skills_command_answers_with_the_skill_list() {
    let (actor, mut actor_rx, _) = actor();

    actor.get_skills().await.unwrap();

    match actor_rx.replies.recv().await {
        Some(CommandReply::SkillsListed(Ok(skills))) => {
            assert!(skills.is_empty());
        }
        other => panic!("expected a SkillsListed event, got {other:?}"),
    }
}

#[tokio::test]
async fn toggle_streaming_answers_with_the_new_value() {
    let (actor, mut actor_rx, initial) = actor();

    actor.toggle_streaming().await.unwrap();

    match actor_rx.replies.recv().await {
        Some(CommandReply::StreamingToggled(Ok(streaming))) => {
            assert_eq!(streaming, !initial.streaming);
        }
        other => panic!("expected a StreamingToggled event, got {other:?}"),
    }
}

#[tokio::test]
async fn info_command_answers_with_the_session_info() {
    let (actor, mut actor_rx, initial) = actor();

    actor.info().await.unwrap();

    match actor_rx.replies.recv().await {
        Some(CommandReply::SessionInfo(Ok(info))) => {
            assert_eq!(info.id, initial.session_id);
        }
        other => panic!("expected a SessionInfo event, got {other:?}"),
    }
}

#[tokio::test]
async fn new_session_command_answers_with_a_snapshot() {
    let (actor, mut actor_rx, _) = actor();

    actor.new_session(None).await.unwrap();

    match actor_rx.replies.recv().await {
        Some(CommandReply::SessionCreated(Ok(snapshot))) => {
            assert!(snapshot.session_id.is_some());
        }
        other => panic!("expected a SessionCreated event, got {other:?}"),
    }
}

#[tokio::test]
async fn a_failing_operation_answers_with_an_error_payload() {
    let (actor, mut actor_rx, _) = actor();

    // an empty session id is rejected by the session store
    actor.load_session(String::new()).await.unwrap();

    match actor_rx.replies.recv().await {
        Some(CommandReply::SessionLoaded(Err(err))) => {
            assert!(
                !err.is_empty(),
                "the error payload should say what went wrong"
            );
        }
        other => panic!("expected a SessionLoaded error, got {other:?}"),
    }
}

#[tokio::test]
async fn replay_history_answers_with_the_history_events() {
    let (actor, mut actor_rx, _) = actor();

    actor.replay_history().await.unwrap();

    match actor_rx.replies.recv().await {
        Some(CommandReply::HistoryReplayed(events)) => {
            assert!(events.is_empty());
        }
        other => panic!("expected a HistoryReplayed event, got {other:?}"),
    }
}

#[tokio::test]
async fn shutdown_answers_with_the_final_snapshot() {
    let (actor, mut actor_rx, initial) = actor();

    let snapshot = HarnessActorClient::shutdown(actor.clone_tx(), &mut actor_rx.events)
        .await
        .expect("shutdown should return the final snapshot");
    assert_eq!(snapshot.session_id, initial.session_id);
}

#[tokio::test]
async fn a_stopped_actor_reports_an_error_for_new_commands() {
    let (actor, mut actor_rx, _) = actor();

    let _ = HarnessActorClient::shutdown(actor.clone_tx(), &mut actor_rx.events).await;

    let err = actor.get_skills().await.unwrap_err();

    assert_eq!(err, "Harness actor stopped");
}

#[tokio::test]
async fn the_tui_opens_the_skill_picker_for_a_skill_list() {
    let mut app = TuiApp::new(Config::new());

    app.handle_command_reply(CommandReply::SkillsListed(Ok(vec![skill("review")])));

    match &app.editor.mode {
        InputMode::Skills { picker } => {
            assert_eq!(picker.rows.len(), 1);
            assert_eq!(picker.rows[0].name, "review");
        }
        _ => panic!("expected the skill picker to be open"),
    }
}

#[tokio::test]
async fn the_tui_reports_a_failed_operation() {
    let mut app = TuiApp::new(Config::new());

    app.handle_command_reply(CommandReply::SessionCreated(Err("no session".to_string())));

    assert_eq!(app.editor.status_visible(), Some(&"no session".to_string()));
}

#[tokio::test]
async fn the_tui_replays_the_history_of_a_loaded_session() {
    let mut app = TuiApp::new(Config::new());
    let snapshot = HarnessSnapshot {
        session_id: Some("session-1".to_string()),
        ..Default::default()
    };
    let followup = app.handle_command_reply(CommandReply::SessionLoaded(Ok(snapshot)));

    assert_eq!(app.snapshot.session_id, Some("session-1".to_string()));
    assert_eq!(
        app.editor.status_visible(),
        Some(&"Loaded session: session-1".to_string())
    );
    // the loaded history is replayed into the UI
    assert!(matches!(followup, Some(HarnessCommand::ReplayHistory)));
}

#[tokio::test]
async fn the_tui_shows_the_session_info() {
    let mut app = TuiApp::new(Config::new());
    let info = SessionInfo {
        id: Some("session-1".to_string()),
        usage: Default::default(),
    };

    app.handle_command_reply(CommandReply::SessionInfo(Ok(info)));

    match &app.editor.mode {
        InputMode::SessionInfo { info } => {
            assert_eq!(info.id, Some("session-1".to_string()));
        }
        _ => panic!("expected the session info to be shown"),
    }
}

#[tokio::test]
async fn the_skills_command_opens_the_picker_when_the_actor_answers() {
    let (actor, mut actor_rx, _) = actor();
    let mut app = TuiApp::new(Config::new());

    // the command only queues the request ...
    let mode = Command::execute(&mut app, &actor, Command::Skills).await;
    app.editor.mode = mode;

    // ... and the answer opens the picker.
    let event = actor_rx.replies.recv().await.unwrap();
    app.handle_command_reply(event);

    assert!(matches!(app.editor.mode, InputMode::Skills { .. }));
}

#[tokio::test]
async fn the_info_command_shows_the_info_when_the_actor_answers() {
    let (actor, mut actor_rx, _) = actor();
    let mut app = TuiApp::new(Config::new());

    let mode = Command::execute(&mut app, &actor, Command::Info).await;
    app.editor.mode = mode;

    let event = actor_rx.replies.recv().await.unwrap();
    app.handle_command_reply(event);

    match &app.editor.mode {
        InputMode::SessionInfo { info } => {
            assert!(info.id.is_some());
        }
        _ => panic!("expected the session info to be shown"),
    }
}
