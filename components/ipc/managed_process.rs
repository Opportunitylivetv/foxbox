/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use ipc_channel::ipc::{ self, IpcOneShotServer, IpcReceiver, IpcSender };

// Assumes Unix
use libc::{self,  c_int};

use serde::{ Deserialize, Serialize };
use std::thread;
use std::thread::JoinHandle;
use std::sync::{ Arc, Mutex, RwLock };
use std::process::Child;
use std::io::{ Error, ErrorKind, Result };

use backoff::Backoff;

/// Unix exit statuses
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub struct ExitStatus(c_int);

const RESTART_TIME_THRESHOLD: u64 = 5; // seconds

pub struct ManagedProcess {
    kill_signal: Arc<Mutex<u32>>,
    pid:         Arc<Mutex<Option<u32>>>,
    thread:      JoinHandle<()>,
    backoff:     Arc<RwLock<Backoff>>,
}

pub unsafe fn fork<F: FnOnce()>(child: F) -> Result<libc::pid_t> {
    match libc::fork() {
        -1 => Err(Error::last_os_error()),
        0  => { child();  unreachable!() },
        pid => Ok(pid)
    }
}

impl ManagedProcess {

    pub fn fork_start<T, F>(f: F) -> Result<()>
        where T: Deserialize + Serialize,
              F: Fn(IpcSender<T>, IpcReceiver<T>) -> Result<()> + Send {

        let (server, server_name): (IpcOneShotServer<T>, String) = IpcOneShotServer::new().unwrap();
        let child_pid = unsafe { fork(|| {
            // Create process local receiver and sender.
            let (tx, rx): (IpcSender<T>, IpcReceiver<T>) = ipc::channel().unwrap();

            // Connect to the parent process and send the local sender
            let server_tx = IpcSender::connect(server_name.clone()).unwrap();
            server_tx.send(tx).unwrap();

            let tx = IpcSender::connect(server_name).unwrap();

            if let Err(e) = f(tx, rx) {
                error!("Process didn't exit cleanly: {}", e);
                // TODO: Use an exit status?
                libc::exit(1);
            } else {
                libc::exit(0);
            }
        }) };

        Ok(())
    }

    /// Create a new ManagedProcess and start it.
    ///
    /// # Examples
    ///
    /// ```
    /// use ipc::ManagedProcess;
    /// use std::process::Command;
    ///
    /// let process = ManagedProcess::start(|| {
    ///     Command::new("echo")
    ///             .arg("Hello")
    ///             .arg("World")
    ///             .spawn()
    /// });
    ///
    /// ```
    pub fn start<F: 'static>(spawn: F) -> Result<ManagedProcess>
        where F: Fn() -> Result<Child> + Send {

        let pid = Arc::new(Mutex::new(None));

        // Uses a u32 Mutex to avoid the compiler complaining that you can use an AtomicBool.
        // In this case we want a bool like thing _and_ a lock.
        let kill_signal = Arc::new(Mutex::new(0));

        let shared_kill_signal  = kill_signal.clone();
        let backoff = Arc::new(RwLock::new(Backoff::from_secs(RESTART_TIME_THRESHOLD)));
        let shared_pid = pid.clone();
        let shared_backoff = backoff.clone();

        let thread = thread::spawn(move || {
            let backoff = shared_backoff;

            loop {
                let mut child_process;

                {
                    let kill_signal = shared_kill_signal.lock().unwrap();
                    let mut pid = shared_pid.lock().unwrap();

                    if *kill_signal == 1 {
                        *pid = None;
                        debug!("Received process kill signal");
                        break;
                    }

                    info!("Starting process. Restarted {} times", checklock!(backoff.read()).get_restart_count());
                    child_process = spawn().unwrap();
                    *pid = Some(child_process.id());
                }

                child_process.wait().unwrap();

                let backoff_duration = checklock!(backoff.write()).next_backoff();
                thread::sleep(backoff_duration);
            }
        });

        Ok(ManagedProcess {
            backoff: backoff,
            kill_signal: kill_signal,
            pid:  pid,
            thread: thread
        })
    }

    #[allow(dead_code)]
    pub fn get_restart_count(&self) -> u64 {
        checklock!(self.backoff.read()).get_restart_count()
    }

    /// Get the current process ID or None if no process is running
    fn get_pid(&self) -> Option<u32> {
        *self.pid.lock().unwrap()
    }

    /// Shut the ManagedProcess down safely. Equivalent to sending SIGKILL to the
    /// running process if it is currently alive
    ///
    /// # Examples
    ///
    /// ```
    /// use ipc::ManagedProcess;
    /// use std::process::Command;
    ///
    /// let process = ManagedProcess::start(|| {
    ///     Command::new("sleep")
    ///             .arg("10000")
    ///             .spawn()
    /// });
    ///
    /// process.unwrap().shutdown().unwrap();
    ///
    /// ```
    pub fn shutdown(self) -> Result<()> {

        {
            let mut kill_signal = self.kill_signal.lock().unwrap();
            *kill_signal = 1;
        }

        // If there is no assigned pid, the process is not running.
        let pid = self.get_pid();

        if pid.is_none() {
            self.join_thread();
            return Ok(());
        }

        let pid = pid.unwrap() as i32;

        // if the process has finished, and therefore had waitpid called,
        // and we kill it, then on unix we might ending up killing a
        // newer process that happens to have a re-used id
        let status_result = try_wait(pid);
        let needs_kill = match status_result {
            Ok(Some(_)) => {
                // Process is already exited
                false
            },
            Ok(None) => {
                // Process is still alive
                true
            },
            Err(e) => {
                // Something went wrong probably at the OS level, warn and don't
                // try and kill the process.
                warn!("{}", e);
                false
            },
        };

        if needs_kill {
            debug!("Sending SIGKILL to pid: {}", pid);
            if let Err(e) = unsafe { c_rv(libc::kill(pid, libc::SIGKILL)) } {
                warn!("{}", e);
            }
        }

        self.join_thread();
        Ok(())
    }

    /// Wait for the thread to exit
    fn join_thread(self) -> () {
        self.thread.join().unwrap();
    }
}

/// A non-blocking 'wait' for a given process id.
fn try_wait(id: i32) -> Result<Option<ExitStatus>> {
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

/// Check the return value of libc function and turn it into a
/// Result type
fn c_rv(t: c_int) -> Result<c_int> {
    if t == -1 {
        Err(Error::last_os_error())
    } else {
        Ok(t)
    }
}

/// Check the return value of a libc function, but, retry the given function if
/// the returned error is EINTR (Interrupted)
fn c_rv_retry<F>(mut f: F) -> Result<c_int>
    where F: FnMut() -> c_int
{
    loop {
        match c_rv(f()) {
            Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
            other => return other,
        }
    }
}


#[test]
fn test_managed_process_restart() {
    use std::time::Duration;
    use std::process::Command;

    let process = ManagedProcess::start(|| {
        Command::new("sleep")
                .arg("0")
                .spawn()
    }).unwrap();

    // Maybe spin with try_recv and check a duration
    // to assert liveness?
    let mut spin_count = 0;
    while process.get_restart_count() < 2 {
        if spin_count > 2 {
            panic!("Process has not restarted twice, within the expected amount of time");
        } else {
            spin_count += 1;
            thread::sleep(Duration::from_secs(3));
        }
    }

    process.shutdown().unwrap();
}

#[test]
fn test_managed_process_shutdown() {
    use std::process::Command;
    // Ideally need a timeout. The test should be, if shutdown doesn't happen immediately,
    // something's broken.
    let process = ManagedProcess::start(|| {
        Command::new("sleep")
                .arg("1000")
                .spawn()
    }).unwrap();

    process.shutdown().unwrap();
}
