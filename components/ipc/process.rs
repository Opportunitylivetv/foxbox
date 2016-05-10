/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use ipc_channel::ipc::{ channel as ipc_channel, IpcOneShotServer, IpcSender, IpcReceiver };
use std::io::{ Error, Result };
use serde::{ Serialize, Deserialize };
use libc;
use utils;
use traits::*;

/// A `Process` with IPC set up such that you can send `TSend` values to the
/// process and receive `TReceive` values from the process.
pub struct Process<TSend, TReceive>
    where TSend: Serialize + Deserialize,
          TReceive: Serialize + Deserialize {
    pid: libc::pid_t,
    exit_status: Option<ExitStatus>,

    channels: (IpcSender<TSend>, IpcReceiver<TReceive>)
}

pub unsafe fn fork<F: FnOnce()>(child: F) -> Result<libc::pid_t> {
    match libc::fork() {
        -1 => Err(Error::last_os_error()),
        0  => { child();  unreachable!() },
        pid => Ok(pid)
    }
}

impl<TSend, TReceive> Process<TSend, TReceive>
    where TSend: Serialize + Deserialize,
          TReceive: Serialize + Deserialize {

    pub fn fork<F>(f: F) -> Result<Process<TSend, TReceive>>
        where F: Fn(IpcSender<TReceive>, IpcReceiver<TSend>) -> Result<()> + Send {

        let (tx_server, tx_server_name): (IpcOneShotServer<IpcSender<TSend>>, String) = IpcOneShotServer::new().unwrap();
        info!("tx ipc: {}", tx_server_name);

        let (rx_server, rx_server_name): (IpcOneShotServer<IpcReceiver<TReceive>>, String) = IpcOneShotServer::new().unwrap();
        debug!("rx ipc: {}", rx_server_name);

        let child_pid = try!(unsafe { fork(|| {
            // Used for parent -> child communication
            let (p_tx, p_rx): (IpcSender<TSend>, IpcReceiver<TSend>) = ipc_channel().unwrap();

            // Connect to the parent process and send the local sender to the parent
            let server_tx = IpcSender::connect(tx_server_name.clone()).unwrap();
            server_tx.send(p_tx).unwrap();

            let (c_tx, c_rx): (IpcSender<TReceive>, IpcReceiver<TReceive>) = ipc_channel().unwrap();

            // Connect to the parent process and use the sender.
            let server_rx = IpcSender::connect(rx_server_name).unwrap();
            server_rx.send(c_rx).unwrap();

            if let Err(e) = f(c_tx, p_rx) {
                error!("Forked process didn't exit cleanly: {}", e);
                // TODO: Use an exit status?
                libc::exit(1);
            } else {
                libc::exit(0);
            }
        }) });

        let (_, tx): (_, IpcSender<TSend>) = tx_server.accept().unwrap();
        let (_, rx): (_, IpcReceiver<TReceive>) = rx_server.accept().unwrap();

        Ok(Process {
            pid: child_pid,
            exit_status: None,
            channels: (tx, rx)
        })
    }
}

impl<TSend, TReceive> ChildProcess for Process<TSend, TReceive>
    where TSend: Serialize + Deserialize,
          TReceive: Serialize + Deserialize {
    fn id(&self) -> u32 {
        self.pid as u32
    }

    fn wait(&mut self) -> Result<ExitStatus> {
        if let Some(exit_status) = self.exit_status {
            return Ok(exit_status);
        }

        let status = try!(utils::wait(self.id() as i32));

        self.exit_status = Some(status);
        Ok(status)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_ipc() {

        let process = Process::fork(|tx, rx| {
            // Different process here!
            let x = rx.recv().unwrap();
            tx.send(x * 2).unwrap();
            Ok(())
        }).unwrap();

        let (tx, rx) = process.channels;

        tx.send(10).unwrap();
        let result = rx.recv().unwrap();

        assert_eq!(20, result);
    }
}

