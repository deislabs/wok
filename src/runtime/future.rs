use std::{
    fs::File,
    future::Future,
    io::BufReader,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
    thread,
};

use crate::wasm::{Runtime, WasiRuntime};

struct RuntimeState {
    completed: bool,
    err: Option<failure::Error>,
    rt: WasiRuntime,
    waker: Option<Waker>,
}

pub struct RuntimeFuture {
    state: Arc<Mutex<RuntimeState>>,
}

impl Future for RuntimeFuture {
    type Output = Result<(BufReader<File>, BufReader<File>), failure::Error>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // get state
        let mut run = self.state.lock().unwrap();
        if run.completed {
            if let Some(e) = run.err.as_ref() {
                Poll::Ready(Err(failure::format_err!(
                    "runtime error: {}",
                    e.to_string()
                )))
            } else {
                Poll::Ready(run.rt.output())
            }
        } else {
            // See https://rust-lang.github.io/async-book/02_execution/03_wakeups.html
            run.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl RuntimeFuture {
    pub fn new(rt: WasiRuntime) -> Self {
        // Start a thread, but track the state of the WASM
        let state = Arc::new(Mutex::new(RuntimeState {
            completed: false,
            rt,
            waker: None,
            err: None,
        }));
        let inner_state = state.clone();
        thread::spawn(move || {
            let mut run = inner_state.lock().unwrap();
            // This runs the WASM to completion.
            // TODO: Once we've tested, reduce this to nice concise if/let
            match run.rt.run() {
                Ok(_) => {
                    // Attach the output? (Not necessary, since it was initialized)
                    //run.out = rt.output();
                    log::info!("Finished executing WASM")
                }
                Err(e) => run.err = Some(e),
            };
            run.completed = true;
            // If there is a waker, take it; if there is a waker, wake it.
            if let Some(waker) = run.waker.take() {
                waker.wake()
            }
        });

        RuntimeFuture { state }
    }

    /// Fetch STDIN and STDOUT for the underlying WASI runtime.
    ///
    /// Normally, these are returned as the output on the future. But in some cases, it may be
    /// necessary to get the logs while the WASM is executing.
    pub fn output(&self) -> Result<(BufReader<File>, BufReader<File>), failure::Error> {
        self.state.lock().unwrap().rt.output()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::collections::HashMap;

    const GREET_FILE: &str = "./testdata/greet.wasm";

    #[tokio::test]
    async fn test_future() {
        let env: HashMap<String, String> = HashMap::new();
        let rt =
            crate::wasm::WasiRuntime::new(GREET_FILE, env, vec![], HashMap::new(), Some("/tmp/"))
                .expect("a new runtime");

        let fut = RuntimeFuture::new(rt);
        fut.await.expect("No error on run");
    }

    #[tokio::test]
    async fn test_future_output_first() {
        let env: HashMap<String, String> = HashMap::new();
        let rt =
            crate::wasm::WasiRuntime::new(GREET_FILE, env, vec![], HashMap::new(), Some("/tmp/"))
                .expect("a new runtime");

        let fut = RuntimeFuture::new(rt);
        let _out = fut.output().expect("should get output");
        fut.await.expect("No error on run");
    }
}
