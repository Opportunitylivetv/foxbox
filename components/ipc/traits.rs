/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use libc::c_int;
use std::io::Result;

/// Unix exit statuses
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub struct ExitStatus(pub c_int);

pub trait ChildProcess {
    fn id(&self) -> u32;
    fn wait(&mut self) -> Result<ExitStatus>;
}


