//! # Robus: the robust robotic bus
//!
//! Robus lets you seamlessly connect robotic actuators and sensors using a unified bus. New modules are automatically detected thanks to the topology detection algorithm.
//!
//! Each robus node is called a module and represent a function in your robot - typically a sensor or actuator. Modules can directly communicate with other modules using broadcast, specific module ids or specific module groups.
//!
//! Robus is lightweighted and designed for the embedded world.
//!
//! Robus reduces the time between an idea to the prototype. It provides a unified messaging achitecture for modular robotics, all modules can be connected on the same 5 wires bus containing both power and 2 communication bus.

#![no_std]
#![feature(alloc)]

#[macro_use(vec)]
extern crate alloc;

#[cfg(target_arch = "arm")]
extern crate stm32f0_hal as hal;
#[cfg(not(target_arch = "arm"))]
extern crate mockup_hal as hal;

use hal::uart;

mod command;
pub use command::Command;

mod module;
pub use module::{Module, ModuleType};

mod msg;
pub use msg::Message;

mod collections;
pub use collections::message_queue;

mod robus_core;
pub use robus_core::Core;

mod registry;
mod recv_buf;

/// Init function to setup robus communication
///
/// Must be called before actually trying to read or send any `Message`.
pub fn init<'a>() -> Core<'a> {
    let mut core = Core::new();

    uart::setup(
        57600,
        uart::NBits::_8bits,
        uart::StopBits::_1b,
        uart::Parity::None,
        |byte| core.receive(byte),
    );

    core
}
