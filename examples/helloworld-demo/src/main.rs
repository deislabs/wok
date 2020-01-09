use std::{thread, time};

fn main() {
    let duration = time::Duration::from_secs(3);

    println!("Hello!");

    loop {
        thread::sleep(duration);
        println!("Hello again!");
    }
}
