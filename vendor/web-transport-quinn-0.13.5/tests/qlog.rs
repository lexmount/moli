//! The `qlog` feature must keep `quinn`'s qlog knobs reachable through our re-export.

#![cfg(feature = "qlog")]

use web_transport_quinn::quinn;

/// `TransportConfig`'s Debug reports whether a qlog sink is installed, which is the
/// only observable state quinn exposes for it.
fn qlog_enabled(transport: &quinn::TransportConfig) -> bool {
    format!("{transport:?}").contains("qlog_stream: true")
}

#[test]
fn transport_config_exposes_qlog() {
    let mut transport = quinn::TransportConfig::default();
    assert!(!qlog_enabled(&transport), "qlog should start disabled");

    let mut config = quinn::QlogConfig::default();
    config.writer(Box::new(std::io::sink()));
    transport.qlog_stream(config.into_stream());

    assert!(
        qlog_enabled(&transport),
        "qlog_stream did not install a sink"
    );
}
