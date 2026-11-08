//! Integration test: Mock Moonraker server + MoonrakerDriver.
//!
//! Tests:
//! 1. Status events arrive when the driver connects.
//! 2. Reconnect works after the mock server drops the connection.

use std::sync::Arc;
use std::time::Duration;

use foreman_core::{
    driver::PrinterDriver,
    drivers::moonraker::{MoonrakerConfig, MoonrakerDriver},
    events::{PrinterState, ServerEvent},
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

/// A minimal mock Moonraker WebSocket server.
/// Responds to JSON-RPC requests and sends a simulated print sequence.
async fn run_mock_moonraker(listener: TcpListener, scenario: MockScenario) {
    while let Ok((stream, _)) = listener.accept().await {
        let scenario = scenario.clone();
        tokio::spawn(async move {
            let ws = tokio_tungstenite::accept_async(stream)
                .await
                .expect("ws accept");
            let (mut tx, mut rx) = ws.split();

            // Respond to incoming RPC requests.
            let mut pending_subscribe = false;
            while let Some(Ok(msg)) = rx.next().await {
                if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                    let req: Value = match serde_json::from_str(&text) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let id = req.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");

                    let response = match method {
                        "server.info" => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "klippy_state": "ready",
                                "hostname": "mock-printer",
                                "moonraker_version": "v0.9.0-mock"
                            }
                        }),
                        "printer.objects.subscribe" => {
                            pending_subscribe = true;
                            json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "eventtime": 1000.0,
                                    "status": {
                                        "print_stats": {
                                            "state": "standby",
                                            "print_duration": 0.0,
                                            "filename": ""
                                        },
                                        "extruder": {
                                            "temperature": 22.5,
                                            "target": 0.0
                                        },
                                        "heater_bed": {
                                            "temperature": 21.0,
                                            "target": 0.0
                                        },
                                        "virtual_sdcard": {
                                            "progress": 0.0,
                                            "file_path": ""
                                        },
                                        "display_status": {
                                            "message": "",
                                            "progress": 0.0
                                        },
                                        "toolhead": {
                                            "position": [0.0, 0.0, 0.0, 0.0],
                                            "homed_axes": ""
                                        },
                                        "webhooks": {
                                            "state": "ready",
                                            "state_message": "Printer is ready"
                                        }
                                    }
                                }
                            })
                        }
                        "printer.objects.query" => {
                            json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "eventtime": 1000.0,
                                    "status": {
                                        "configfile": {
                                            "settings": {
                                                "extruder": {},
                                                "stepper_x": {}
                                            }
                                        },
                                        "toolhead": {
                                            "axis_maximum": [120.0, 120.0, 120.0, 0.0],
                                            "axis_minimum": [0.0, 0.0, 0.0, 0.0]
                                        }
                                    }
                                }
                            })
                        }
                        "server.webcams.list" => {
                            json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "webcams": [{
                                        "name": "default",
                                        "stream_url": "http://mock:8080/webcam",
                                        "snapshot_url": "http://mock:8080/webcam?action=snapshot"
                                    }]
                                }
                            })
                        }
                        "printer.objects.list" => {
                            json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "objects": [
                                        "extruder",
                                        "heater_bed",
                                        "toolhead",
                                        "gcode_macro START_PRINT",
                                        "gcode_macro END_PRINT",
                                        "gcode_macro _INTERNAL_MACRO"
                                    ]
                                }
                            })
                        }
                        _ => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32601, "message": "Method not found" }
                        }),
                    };

                    let _ = tx
                        .send(tokio_tungstenite::tungstenite::Message::Text(
                            serde_json::to_string(&response).unwrap().into(),
                        ))
                        .await;

                    // After all initial handshake, send the print scenario.
                    if pending_subscribe && method == "printer.objects.list" {
                        pending_subscribe = false;
                        // Small delay for the driver to process.
                        tokio::time::sleep(Duration::from_millis(100)).await;

                        for update in &scenario.updates {
                            let notification = json!({
                                "jsonrpc": "2.0",
                                "method": "notify_status_update",
                                "params": [update, 1001.0]
                            });
                            let _ = tx
                                .send(tokio_tungstenite::tungstenite::Message::Text(
                                    serde_json::to_string(&notification).unwrap().into(),
                                ))
                                .await;
                            tokio::time::sleep(Duration::from_millis(600)).await;
                        }

                        if scenario.drop_after_updates {
                            // Close connection to test reconnect.
                            let _ = tx.close().await;
                            return;
                        }
                    }
                }
            }
        });
    }
}

/// Describes what the mock server should do.
#[derive(Clone)]
struct MockScenario {
    updates: Vec<Value>,
    drop_after_updates: bool,
}

fn short_print_scenario() -> MockScenario {
    MockScenario {
        updates: vec![
            // Heating up.
            json!({
                "extruder": { "temperature": 180.0, "target": 210.0 },
                "heater_bed": { "temperature": 45.0, "target": 60.0 }
            }),
            // Printing starts.
            json!({
                "print_stats": { "state": "printing", "filename": "test.gcode", "print_duration": 10.0 },
                "extruder": { "temperature": 210.0 },
                "heater_bed": { "temperature": 60.0 },
                "virtual_sdcard": { "progress": 0.25 }
            }),
            // Mid-print.
            json!({
                "virtual_sdcard": { "progress": 0.75 },
                "print_stats": { "print_duration": 120.0 }
            }),
            // Print complete.
            json!({
                "print_stats": { "state": "complete", "print_duration": 180.0 },
                "virtual_sdcard": { "progress": 1.0 }
            }),
        ],
        drop_after_updates: false,
    }
}

#[tokio::test]
async fn status_events_arrive_from_mock() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Start mock server.
    tokio::spawn(run_mock_moonraker(listener, short_print_scenario()));

    // Create event bus.
    let (bus_tx, mut bus_rx) = broadcast::channel::<ServerEvent>(64);
    let bus_tx = Arc::new(bus_tx);

    // Create and connect driver.
    let config = MoonrakerConfig {
        printer_id: 1,
        url: format!("http://127.0.0.1:{}", addr.port()),
        api_key: None,
    };
    let driver = MoonrakerDriver::new(config, Arc::clone(&bus_tx));
    let caps = driver.connect().await.expect("connect");

    // Verify capabilities.
    assert_eq!(caps.bed_size, Some((120.0, 120.0)));
    assert_eq!(caps.extruder_count, 1);
    assert_eq!(caps.webcams.len(), 1);
    assert_eq!(caps.webcams[0].name, "default");
    assert!(caps.macros.contains(&"END_PRINT".to_owned()));
    assert!(caps.macros.contains(&"START_PRINT".to_owned()));
    // Internal macros (starting with _) should be filtered out.
    assert!(!caps.macros.contains(&"_INTERNAL_MACRO".to_owned()));

    // Collect events for a few seconds.
    let mut statuses = Vec::new();
    let mut saw_connection = false;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        tokio::select! {
            ev = bus_rx.recv() => {
                match ev {
                    Ok(ServerEvent::PrinterStatus { status, .. }) => {
                        statuses.push(status);
                    }
                    Ok(ServerEvent::PrinterConnection { event, .. }) => {
                        if event.kind == foreman_core::events::ConnectionEventKind::Online {
                            saw_connection = true;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            _ = tokio::time::sleep_until(deadline) => break,
        }
    }

    // We should have received a connection event.
    assert!(
        saw_connection,
        "should have seen an Online connection event"
    );

    // We should have received multiple status events.
    assert!(
        statuses.len() >= 3,
        "expected at least 3 status events, got {}",
        statuses.len()
    );

    // At some point we should have seen the printer printing.
    let any_printing = statuses.iter().any(|s| s.state == PrinterState::Printing);
    assert!(any_printing, "should have seen Printing state");

    // Nozzle temp should have been > 0 at some point.
    let any_hot = statuses
        .iter()
        .any(|s| s.nozzle.as_ref().is_some_and(|n| n.actual > 100.0));
    assert!(any_hot, "should have seen nozzle > 100°C");

    driver.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn reconnect_after_drop() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Scenario that drops connection after updates.
    let scenario = MockScenario {
        updates: vec![json!({
            "extruder": { "temperature": 50.0, "target": 0.0 }
        })],
        drop_after_updates: true,
    };

    tokio::spawn(run_mock_moonraker(listener, scenario));

    let (bus_tx, mut bus_rx) = broadcast::channel::<ServerEvent>(64);
    let bus_tx = Arc::new(bus_tx);

    let config = MoonrakerConfig {
        printer_id: 2,
        url: format!("http://127.0.0.1:{}", addr.port()),
        api_key: None,
    };
    let driver = MoonrakerDriver::new(config, Arc::clone(&bus_tx));
    driver.connect().await.expect("connect");

    // Wait and collect events — should see online, then offline (after drop), then online again (reconnect).
    let mut connection_events = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);

    loop {
        tokio::select! {
            ev = bus_rx.recv() => {
                match ev {
                    Ok(ServerEvent::PrinterConnection { event, .. }) => {
                        connection_events.push(event.kind.clone());
                        // After seeing offline + a second online, we're done.
                        if connection_events.len() >= 3 {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            _ = tokio::time::sleep_until(deadline) => break,
        }
    }

    // Expect: Online, Offline, Online (reconnect).
    assert!(
        connection_events.len() >= 3,
        "expected at least 3 connection events (Online, Offline, Online), got {:?}",
        connection_events
    );

    use foreman_core::events::ConnectionEventKind::*;
    assert_eq!(connection_events[0], Online);
    assert_eq!(connection_events[1], Offline);
    assert_eq!(connection_events[2], Online);

    driver.disconnect().await.expect("disconnect");
}
