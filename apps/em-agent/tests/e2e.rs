#[cfg(unix)]
mod unix {
    use std::{path::Path, process::Stdio, time::Duration};

    use em_core::{
        AgentRequest, AgentResponse, LoginCredentialsDto, OperationKind, OperationState,
        TokenOperationRequest,
    };
    use tokio::process::Command;
    use uuid::Uuid;

    #[tokio::test]
    async fn completes_session_and_multi_device_operation_over_ipc() {
        let endpoint = format!("/tmp/em-station/e2e-{}.sock", Uuid::new_v4());
        let mut agent = Command::new(env!("CARGO_BIN_EXE_em-agent"))
            .env("EM_AGENT_ENDPOINT", &endpoint)
            .env("EM_AGENT_MODE", "simulator")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("agent should start");

        for _ in 0..50 {
            if Path::new(&endpoint).exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        let login = em_protocol::request(
            &endpoint,
            &AgentRequest::Login(LoginCredentialsDto {
                username: "integration.user".into(),
                password: "temporary-test-secret".into(),
            }),
        )
        .await
        .expect("login request should complete");
        assert!(matches!(login, AgentResponse::Session(Some(_))));

        let devices = match em_protocol::request(&endpoint, &AgentRequest::ListDevices)
            .await
            .expect("device request should complete")
        {
            AgentResponse::Devices(devices) => devices,
            response => panic!("unexpected device response: {response:?}"),
        };
        assert_eq!(devices.len(), 2);

        let operation_id = match em_protocol::request(
            &endpoint,
            &AgentRequest::StartOperation {
                kind: OperationKind::Install,
                request: TokenOperationRequest {
                    device_ids: devices.into_iter().map(|device| device.id).collect(),
                    mode_ids: vec!["MODE_ENGINEER".into()],
                    expires_at: None,
                },
            },
        )
        .await
        .expect("operation request should complete")
        {
            AgentResponse::Operation(operation) => operation.id,
            response => panic!("unexpected operation response: {response:?}"),
        };

        let final_status = loop {
            let response =
                em_protocol::request(&endpoint, &AgentRequest::GetOperation { operation_id })
                    .await
                    .expect("operation polling should complete");
            let AgentResponse::Operation(operation) = response else {
                panic!("unexpected polling response: {response:?}");
            };
            if matches!(
                operation.state,
                OperationState::Completed | OperationState::Failed | OperationState::Cancelled
            ) {
                break operation;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };

        assert_eq!(final_status.state, OperationState::Completed);
        assert_eq!(final_status.completed, 2);
        assert!(final_status.results.iter().all(|result| result.success));

        agent.kill().await.expect("agent should stop");
        let _ = tokio::fs::remove_file(endpoint).await;
    }
}
