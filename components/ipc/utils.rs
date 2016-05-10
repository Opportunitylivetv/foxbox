/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use ipc_channel::ipc::{ self, IpcOneShotServer, IpcReceiver, IpcSender };
use libc::{ self, c_int };
use std::io::{ Error, ErrorKind, Result };

use traits::ExitStatus;

/// A non-blocking 'wait' for a given process id.
pub fn try_wait(id: i32) -> Result<Option<ExitStatus>> {
    let mut status = 0 as c_int;

    match c_rv_retry(|| unsafe {
        libc::waitpid(id, &mut status, libc::WNOHANG)
    }) {
        Ok(0)  => Ok(None),
        Ok(n) if n == id => Ok(Some(ExitStatus(status))),
        Ok(n)  => Err(Error::new(ErrorKind::NotFound, format!("Unknown pid: {}", n))),
        Err(e) => Err(Error::new(ErrorKind::Other, format!("Unknown waitpid error: {}", e)))
    }
}

pub fn wait(id: i32) -> Result<ExitStatus> {
    let mut status = 0;
    try!(c_rv_retry(|| unsafe { libc::waitpid(id, &mut status, 0) }));

    Ok(ExitStatus(status))
}

/// Check the return value of libc function and turn it into a
/// Result type
pub fn c_rv(t: c_int) -> Result<c_int> {
    if t == -1 {
        Err(Error::last_os_error())
    } else {
        Ok(t)
    }
}

/// Check the return value of a libc function, but, retry the given function if
/// the returned error is EINTR (Interrupted)
pub fn c_rv_retry<F>(mut f: F) -> Result<c_int>
    where F: FnMut() -> c_int
{
    loop {
        match c_rv(f()) {
            Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
            other => return other,
        }
    }
}
