#[cfg(not(target_os = "macos"))]
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
#[cfg(not(target_os = "macos"))]
use std::sync::atomic::Ordering;

use anyhow::{Context, Result, bail};
use rdev::Key;
#[cfg(not(target_os = "macos"))]
use rdev::{Event, EventType};
use tokio::sync::mpsc;
#[cfg(not(target_os = "macos"))]
use tracing::{error, info};
#[cfg(not(target_os = "macos"))]
use usahp_core::Action;
use usahp_core::{InputKind, Mapping};

use crate::broker::BrokerCommand;
#[cfg(not(target_os = "macos"))]
use crate::broker::PhysicalEvent;

pub fn validate(mappings: &[Mapping]) -> Result<()> {
    for mapping in mappings {
        match mapping.input {
            InputKind::Keyboard => {
                parse_key(&mapping.code).with_context(|| format!("mapping '{}'", mapping.id))?;
                if mapping.device.is_some() {
                    bail!("keyboard mapping '{}' must not set device", mapping.id);
                }
            }
            InputKind::Gamepad => validate_gamepad(mapping)?,
        }
    }
    Ok(())
}

pub fn spawn(
    mappings: &[Mapping],
    broker: mpsc::Sender<BrokerCommand>,
    capture: Arc<AtomicBool>,
) -> Result<()> {
    let keyboard: Vec<_> = mappings
        .iter()
        .filter(|mapping| mapping.input == InputKind::Keyboard)
        .cloned()
        .collect();
    if !keyboard.is_empty() {
        spawn_keyboard(keyboard, broker.clone(), capture.clone())?;
    }
    spawn_gamepads(mappings, broker, capture)?;
    Ok(())
}

fn spawn_keyboard(
    mappings: Vec<Mapping>,
    broker: mpsc::Sender<BrokerCommand>,
    capture: Arc<AtomicBool>,
) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        // rdev::grab crashes on macOS (TSM off-main-thread → SIGTRAP). Use a
        // native CGEventTap that reads only keycodes — no TextServices.
        crate::macos_keyboard::spawn(&mappings, broker, capture);
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let mut by_key: HashMap<Key, Vec<String>> = HashMap::new();
        for mapping in mappings {
            by_key
                .entry(parse_key(&mapping.code)?)
                .or_default()
                .push(mapping.id);
        }
        std::thread::Builder::new()
            .name("usahp-keyboard-grab".into())
            .spawn(move || {
                info!("keyboard suppression backend started");
                let callback = move |event: Event| -> Option<Event> {
                    // Capture released: pass the event through to the OS untouched.
                    if !capture.load(Ordering::Relaxed) {
                        return Some(event);
                    }
                    let edge = match &event.event_type {
                        EventType::KeyPress(key) => Some((*key, Action::Pressed)),
                        EventType::KeyRelease(key) => Some((*key, Action::Released)),
                        _ => None,
                    };
                    let Some((key, action)) = edge else {
                        return Some(event);
                    };
                    let Some(mapping_ids) = by_key.get(&key) else {
                        return Some(event);
                    };
                    for mapping_id in mapping_ids {
                        if broker
                            .blocking_send(BrokerCommand::Input(PhysicalEvent {
                                mapping_id: mapping_id.clone(),
                                action,
                            }))
                            .is_err()
                        {
                            return Some(event);
                        }
                    }
                    None
                };
                if let Err(error) = rdev::grab(callback) {
                    error!(?error, "keyboard grab failed; check platform permissions");
                }
            })
            .context("could not start keyboard suppression thread")?;
        Ok(())
    }
}

fn parse_key(code: &str) -> Result<Key> {
    let key = match code.to_ascii_lowercase().as_str() {
        "space" => Key::Space,
        "return" | "enter" => Key::Return,
        "escape" | "esc" => Key::Escape,
        "tab" => Key::Tab,
        "up" | "uparrow" => Key::UpArrow,
        "down" | "downarrow" => Key::DownArrow,
        "left" | "leftarrow" => Key::LeftArrow,
        "right" | "rightarrow" => Key::RightArrow,
        "a" => Key::KeyA,
        "b" => Key::KeyB,
        "c" => Key::KeyC,
        "d" => Key::KeyD,
        "e" => Key::KeyE,
        "f" => Key::KeyF,
        "g" => Key::KeyG,
        "h" => Key::KeyH,
        "i" => Key::KeyI,
        "j" => Key::KeyJ,
        "k" => Key::KeyK,
        "l" => Key::KeyL,
        "m" => Key::KeyM,
        "n" => Key::KeyN,
        "o" => Key::KeyO,
        "p" => Key::KeyP,
        "q" => Key::KeyQ,
        "r" => Key::KeyR,
        "s" => Key::KeyS,
        "t" => Key::KeyT,
        "u" => Key::KeyU,
        "v" => Key::KeyV,
        "w" => Key::KeyW,
        "x" => Key::KeyX,
        "y" => Key::KeyY,
        "z" => Key::KeyZ,
        _ => bail!("unsupported keyboard code '{code}'"),
    };
    Ok(key)
}

#[cfg(not(target_os = "linux"))]
fn validate_gamepad(mapping: &Mapping) -> Result<()> {
    bail!(
        "gamepad mapping '{}' cannot guarantee suppression on {}; use Linux evdev or remove it",
        mapping.id,
        std::env::consts::OS
    )
}

#[cfg(not(target_os = "linux"))]
fn spawn_gamepads(
    _: &[Mapping],
    _: mpsc::Sender<BrokerCommand>,
    _capture: Arc<AtomicBool>,
) -> Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_gamepad(mapping: &Mapping) -> Result<()> {
    if mapping.device.as_deref().unwrap_or_default().is_empty() {
        bail!(
            "gamepad mapping '{}' requires an evdev device path",
            mapping.id
        );
    }
    mapping.code.parse::<u16>().with_context(|| {
        format!(
            "gamepad mapping '{}' code must be a numeric evdev key code",
            mapping.id
        )
    })?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn spawn_gamepads(
    mappings: &[Mapping],
    broker: mpsc::Sender<BrokerCommand>,
    capture: Arc<AtomicBool>,
) -> Result<()> {
    use evdev::{Device, EventType};

    let mut by_device: HashMap<String, HashMap<u16, Vec<String>>> = HashMap::new();
    for mapping in mappings.iter().filter(|m| m.input == InputKind::Gamepad) {
        by_device
            .entry(mapping.device.clone().unwrap())
            .or_default()
            .entry(mapping.code.parse()?)
            .or_default()
            .push(mapping.id.clone());
    }

    for (path, codes) in by_device {
        let broker = broker.clone();
        let capture = capture.clone();
        let error_path = path.clone();
        std::thread::Builder::new()
            .name(format!("usahp-evdev-{path}"))
            .spawn(move || {
                let mut device = match Device::open(&path) {
                    Ok(device) => device,
                    Err(error) => {
                        error!(%path, %error, "could not open gamepad evdev device");
                        return;
                    }
                };
                if let Err(error) = device.grab() {
                    error!(%path, %error, "could not exclusively grab gamepad");
                    return;
                }
                info!(%path, "exclusive gamepad backend started");
                loop {
                    let events = match device.fetch_events() {
                        Ok(events) => events,
                        Err(error) => {
                            error!(%path, %error, "gamepad read failed");
                            break;
                        }
                    };
                    for event in events {
                        // Capture released: skip routing. NOTE the device stays
                        // exclusively grabbed, so other apps still don't see these
                        // events — full release would need device.ungrab().
                        if !capture.load(Ordering::Relaxed) {
                            continue;
                        }
                        if event.event_type() != EventType::KEY {
                            continue;
                        }
                        let Some(mapping_ids) = codes.get(&event.code()) else {
                            continue;
                        };
                        let action = match event.value() {
                            1 => Action::Pressed,
                            0 => Action::Released,
                            _ => continue,
                        };
                        for mapping_id in mapping_ids {
                            if broker
                                .blocking_send(BrokerCommand::Input(PhysicalEvent {
                                    mapping_id: mapping_id.clone(),
                                    action,
                                }))
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                }
            })
            .with_context(|| format!("could not start gamepad thread for {error_path}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_documented_keyboard_codes() {
        for code in ["Space", "Return", "Escape", "A", "Z", "LeftArrow"] {
            assert!(parse_key(code).is_ok(), "{code}");
        }
    }
}
