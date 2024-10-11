#![feature(core_io_borrowed_buf)]
use std::{env, error::Error};

use client::Client;
use server::Server;

mod client;
mod server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();

    let run_as = &args[1];

    if run_as == "client" {
        let client = Client::init("10.0.0.139".to_string(), 8888);
        client.copy_folder("/Users/vikaspathania/Documents/MyTasks");
        Ok(())
    } else if run_as == &"server".to_string() {
        let ip = "127.0.0.1".to_string();
        let port = 8888;

        let server = Server::init(ip, port);
        // server.run().await;
        Result::Ok(())
    } else {
        print!("Enter either client or server eg. cargo run client OR cargo run server");
        Result::Ok(())
    }
}
