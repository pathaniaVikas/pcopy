// #![feature(core_io_borrowed_buf)]
#![feature(ip_as_octets)]
mod client;
use crate::{client::client::Client, relay::relay::ShareableServerHandle};

mod server;
use crate::server::server::Server;

mod relay;
use local_ip_address::local_ip;
use std::{env, path::Path};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        print!(
            "Provide argument `server/client/relay` eg.\n`cargo run server`\nor\n`cargo run client`"
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
        let my_local_ip = local_ip().unwrap();
        let port = 9999;
        let server = Server::init(my_local_ip.to_string(), port);
        server.run().await?;
        Result::Ok(())
    } else if run_as.to_lowercase() == "relay".to_string() {
        let my_local_ip = local_ip().unwrap();
        let port = 6666;
        let server = ShareableServerHandle::init(my_local_ip, port);
        server.run().await?;
        Result::Ok(())
    } else {
        print!("Enter either client/server/relay eg. `cargo run client <folder_path> <ip_address>` OR `cargo run server`");
        Result::Ok(())
    }
}
