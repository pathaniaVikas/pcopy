#![feature(core_io_borrowed_buf)]

use client::Client;
use server::Server;
use std::{env, error::Error, path::Path};
use tokio::{io::AsyncReadExt, net::TcpListener};
use tracing::{error, info};

mod client;
mod server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(false)
        .init();

    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        print!(
            "Provide argument `server` or `client` eg.\n`cargo run server`\nor\n`cargo run client`"
        );
        return Result::Ok(());
    }

    let run_as = &args[1];

    if run_as.to_lowercase() == "client" {
        // Read path from terminal input
        if args.len() < 3 {
            print!(
                "`client` needs a folder path to copy to eg.\n`cargo run client \"/Users/vikaspathania/Documents/books\" <ip_address>`"
            );
            return Result::Ok(());
        }
        let path = &args[2];

        // Read server ip from terminal input
        if args.len() < 4 {
            print!(
                "`client` needs a server ip to copy to eg.\n`cargo run client <folder_path> 10.0.0.139`"
            );
            return Result::Ok(());
        }
        let server_ip = &args[3];
        let mut client = Client::init(server_ip.to_string(), 8888);
        client.send_folder(Path::new(path));
        Ok(())
    } else if run_as.to_lowercase() == "server".to_string() {
        let ip = "127.0.0.1".to_string();
        let port = 8888;
        let server = Server::init(ip, port);
        server.run().await;
        Result::Ok(())
    } else {
        print!("Enter either client or server eg. cargo run client OR cargo run server");
        Result::Ok(())
    }
}
