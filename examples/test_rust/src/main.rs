use orionis_agent::{orion_trace, install_panic_hook, start, agent};
use std::time::Duration;
use std::thread;

fn buggy_function() {
    orion_trace!();
    println!("Handling Rust request...");
    let slice = vec![1, 2, 3];
    // Deliberate panic: index out of bounds
    let _ = slice[10];
}

fn entry_point() {
    orion_trace!();
    buggy_function();
}

fn main() {
    // Start Orionis agent to connect to localhost:7700
    start("http://localhost:7700");
    
    // Install the panic hook to automatically capture and flush traces on panic
    if let Some(ag) = agent() {
        install_panic_hook(ag.clone());
    }

    println!("Starting Rust process... Will panic shortly!");

    // Let the tracing sender thread start up
    thread::sleep(Duration::from_millis(500));

    // Will panic inside this function
    entry_point();

    // The panic hook will flush the events automatically.
}
