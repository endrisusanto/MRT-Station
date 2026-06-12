#[cfg(unix)]
#[tokio::test]
async fn protocol_client_reports_connection_failure_cleanly() {
    let response = em_protocol::request(
        "/tmp/em-station/missing.sock",
        &em_core::AgentRequest::Health,
    )
    .await;
    assert!(response.is_err());
}
