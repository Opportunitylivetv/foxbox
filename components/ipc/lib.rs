/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

#![feature(plugin)]

#![plugin(clippy)]
#![deny(clippy)]

extern crate ipc_channel;
extern crate libc;
#[macro_use]
extern crate log;
extern crate serde;
extern crate serde_json;

macro_rules! checklock (
    ($e: expr) => {
        match $e {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
);

mod backoff;
mod managed_process;
mod traits;
mod process;
mod utils;

pub use managed_process::*;
pub use process::*;
