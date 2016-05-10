/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

// Assumes Unix
use libc::{self,  c_int};

use serde::{ Deserialize, Serialize };
use std::thread;
use std::thread::JoinHandle;
use std::sync::{ Arc, Mutex, RwLock };
use std::process::Child;
use std::io::{ Error, ErrorKind, Result };

use backoff::Backoff;
use traits::{ ChildProcess, ExitStatus };
use utils::*;

const RESTART_TIME_THRESHOLD: u64 = 5; // seconds

impl ChildProcess for Child {
    fn id(&self) -> u32 {
        Child::id(self)
    }

    fn wait(&mut self) -> Result<ExitStatus> {
        let exit_status = try!(Child::wait(self));

        if let Some(status) = exit_status.code() {
            Ok(ExitStatus(status))
        } else {
            // None zero because std::process::ExitStatus::code returns
            // None if the process was terminated by a signal
            Ok(ExitStatus(1))
        }
    }
}

pub struct ManagedProcess {
    kill_signal: Arc<Mutex<u32>>,
    pid:         Arc<Mutex<Option<u32>>>,
    thread:      JoinHandle<()>,
    backoff:     Arc<RwLock<Backoff>>,
}

impl ManagedProcess {

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
    pub fn start<TChild, F: 'static>(spawn: F) -> Result<ManagedProcess>
        where TChild: ChildProcess,
              F: Fn() -> Result<TChild> + Send {

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
