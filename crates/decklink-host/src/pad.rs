//! ViGEm Xbox 360 wired virtual pad.

use anyhow::{Context, Result};
use decklink_hid::{GamepadButtons, GamepadReport, Hat};
use vigem_client::{Client, TargetId, XButtons, XGamepad, Xbox360Wired};

pub struct VirtualPad {
    target: Xbox360Wired<Client>,
}

impl VirtualPad {
    pub fn new() -> Result<Self> {
        let client = Client::connect().context("connect ViGEmBus")?;
        let mut target = Xbox360Wired::new(client, TargetId::XBOX360_WIRED);
        target.plugin().context("plugin virtual Xbox 360")?;
        target.wait_ready().context("wait_ready")?;
        Ok(Self { target })
    }

    pub fn reset(&mut self) {
        let _ = self.target.update(&XGamepad::default());
    }

    pub fn update(&mut self, r: &GamepadReport) {
        let mut raw: u16 = 0;
        if r.buttons.contains(GamepadButtons::A) {
            raw |= XButtons::A;
        }
        if r.buttons.contains(GamepadButtons::B) {
            raw |= XButtons::B;
        }
        if r.buttons.contains(GamepadButtons::X) {
            raw |= XButtons::X;
        }
        if r.buttons.contains(GamepadButtons::Y) {
            raw |= XButtons::Y;
        }
        if r.buttons.contains(GamepadButtons::L1) {
            raw |= XButtons::LB;
        }
        if r.buttons.contains(GamepadButtons::R1) {
            raw |= XButtons::RB;
        }
        if r.buttons.contains(GamepadButtons::SELECT) {
            raw |= XButtons::BACK;
        }
        if r.buttons.contains(GamepadButtons::START) {
            raw |= XButtons::START;
        }
        if r.buttons.contains(GamepadButtons::L3) {
            raw |= XButtons::LTHUMB;
        }
        if r.buttons.contains(GamepadButtons::R3) {
            raw |= XButtons::RTHUMB;
        }
        if r.buttons.contains(GamepadButtons::GUIDE) {
            raw |= XButtons::GUIDE;
        }
        match r.hat {
            Hat::North | Hat::NorthEast | Hat::NorthWest => raw |= XButtons::UP,
            _ => {}
        }
        match r.hat {
            Hat::South | Hat::SouthEast | Hat::SouthWest => raw |= XButtons::DOWN,
            _ => {}
        }
        match r.hat {
            Hat::West | Hat::NorthWest | Hat::SouthWest => raw |= XButtons::LEFT,
            _ => {}
        }
        match r.hat {
            Hat::East | Hat::NorthEast | Hat::SouthEast => raw |= XButtons::RIGHT,
            _ => {}
        }

        // XInput: +Y is up; Deck/HID often use +Y down — flip ly/ry.
        let gamepad = XGamepad {
            buttons: XButtons { raw },
            left_trigger: r.lt,
            right_trigger: r.rt,
            thumb_lx: r.lx,
            thumb_ly: r.ly.saturating_neg(),
            thumb_rx: r.rx,
            thumb_ry: r.ry.saturating_neg(),
        };
        let _ = self.target.update(&gamepad);
    }
}
