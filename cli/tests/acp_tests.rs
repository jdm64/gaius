use agent_client_protocol::{
    Agent, Channel, Client, ConnectTo,
    schema::ProtocolVersion,
    v1::schema::InitializeRequest as InitializeRequestV1,
    v2::InitializeOutcome,
    v2::schema::{
        CancelSessionNotification, ClientNotification, ClientRequest, Implementation,
        InitializeRequest, NewSessionRequest,
    },
};
use gaius::{
    acp::{
        message::{ACPNotification, ACPRequest},
        message_v2::InitializeV2,
        setup_v1::ACPSetupV1,
        setup_v2::ACPSetupV2,
    },
    config::Config,
};
use std::{sync::Arc, time::Duration};

/// The agent under test only speaks protocol v2.
fn setup() -> ACPSetupV2 {
    ACPSetupV2::new(Arc::new(Config::default()))
}

fn setup_v1() -> ACPSetupV1 {
    ACPSetupV1::new(Arc::new(Config::default()))
}

#[tokio::test]
async fn v1_client_gets_an_unsupported_protocol_version_error() {
    let (client, agent) = Channel::duplex();

    let server = tokio::spawn(async move {
        Agent
            .protocol_router()
            .with_v2(setup())
            .connect_to(agent)
            .await
    });

    let outcome = Client
        .v1()
        .connect_with(client, async |cx| {
            cx.send_request(InitializeRequestV1::new(ProtocolVersion::V1))
                .block_task()
                .await
        })
        .await;

    // A rejected initialize leaves the connection open, so the client sees
    // the error and not a broken connection.
    let error = match outcome {
        Ok(_) => panic!("a v1 initialize is not supported"),
        Err(error) => error,
    };
    assert_eq!(i32::from(error.code), -32022);

    let data = error.data.expect("the error names the supported versions");
    assert_eq!(data["requested"], serde_json::json!(1));
    assert_eq!(data["supported"], serde_json::json!([2]));

    // The client is done, so the connection closes and the agent shuts down.
    let _ = tokio::time::timeout(Duration::from_secs(5), server).await;
}

#[tokio::test]
async fn v1_client_routes_to_the_v1_handler() {
    let (client, agent) = Channel::duplex();

    let server = tokio::spawn(async move {
        Agent
            .protocol_router()
            .with_v1(setup_v1())
            .with_v2(setup())
            .connect_to(agent)
            .await
    });

    let response = Client
        .v1()
        .connect_with(client, async |cx| {
            cx.send_request(InitializeRequestV1::new(ProtocolVersion::V1))
                .block_task()
                .await
        })
        .await
        .expect("v1 client initializes with v1");
    assert_eq!(response.protocol_version, ProtocolVersion::V1);
    assert_eq!(response.agent_info.expect("agent info").name, "gaius");
    let capabilities = response.agent_capabilities;
    assert!(capabilities.session_capabilities.list.is_some());
    assert!(capabilities.session_capabilities.resume.is_some());
    assert!(capabilities.session_capabilities.close.is_some());
    assert!(capabilities.session_capabilities.delete.is_some());
    assert!(capabilities.load_session);

    let _ = tokio::time::timeout(Duration::from_secs(5), server).await;
}

#[tokio::test]
async fn v2_client_initializes_with_the_v2_version() {
    let (client, agent) = Channel::duplex();

    let server = tokio::spawn(async move {
        Agent
            .protocol_router()
            .with_v2(setup())
            .connect_to(agent)
            .await
    });

    let outcome = Client
        .v2()
        .connect_with(client, async |cx| {
            cx.send_initialize(InitializeRequest::new(
                ProtocolVersion::V2,
                Implementation::new("test-client", "0.1.0"),
            ))
            .block_task()
            .await
        })
        .await;

    let outcome = outcome.expect("the connection stays open");
    let InitializeOutcome::Initialized(response) = outcome else {
        panic!("the agent speaks v2, so the client initializes with v2");
    };
    assert_eq!(response.protocol_version, ProtocolVersion::V2);
    assert_eq!(response.info.name, "gaius");

    // The client is done, so the connection closes and the agent shuts down.
    let _ = tokio::time::timeout(Duration::from_secs(5), server).await;
}

#[test]
fn initialize_rejects_a_protocol_version_the_agent_does_not_speak() {
    assert!(InitializeV2::check_version(ProtocolVersion::V2).is_ok());

    let error = InitializeV2::check_version(ProtocolVersion::V1).expect_err("v1 is not spoken");
    assert_eq!(i32::from(error.code), -32022);

    let data = error.data.expect("the error names the supported versions");
    assert_eq!(data["requested"], serde_json::json!(1));
    assert_eq!(data["supported"], serde_json::json!([2]));
}

#[test]
fn v2_requests_dispatch_to_the_neutral_request_types() {
    let cwd = std::path::PathBuf::from("/tmp");
    let requests = [
        (
            ClientRequest::InitializeRequest(Box::new(InitializeRequest::new(
                ProtocolVersion::V2,
                Implementation::new("test-client", "0.1.0"),
            ))),
            "initialize",
        ),
        (
            ClientRequest::NewSessionRequest(Box::new(NewSessionRequest::new(cwd))),
            "session/new",
        ),
    ];

    for (request, method) in requests {
        let dispatched = ACPRequest::from(request);
        let name = match &dispatched {
            ACPRequest::Initialize(_) => "initialize",
            ACPRequest::ListSessions(_) => "session/list",
            ACPRequest::NewSession(_) => "session/new",
            ACPRequest::ResumeSession(_) => "session/resume",
            ACPRequest::CloseSession(_) => "session/close",
            ACPRequest::DeleteSession(_) => "session/delete",
            ACPRequest::SetConfig(_) => "session/set_config_option",
            ACPRequest::Prompt(_) => "session/prompt",
            ACPRequest::Other => "other",
        };
        assert_eq!(name, method);
    }
}

#[test]
fn v2_notifications_dispatch_to_the_neutral_notification_types() {
    let cancel = ClientNotification::CancelSessionNotification(Box::new(
        CancelSessionNotification::new("session-1"),
    ));
    assert!(matches!(
        ACPNotification::from(cancel),
        ACPNotification::CancelSession
    ));
}
