use std::{
    fs::{self, DirEntry},
    io::{self, Read, Write},
    net::TcpStream,
    path::Path,
};

pub struct Client {
    server_ip: String,
    server_port: u32,
}

impl Client {
    pub fn init(server_ip: String, server_port: u32) -> Self {
        Self {
            server_ip: server_ip.clone(),
            server_port,
        }
    }

    pub fn copy_folder(self, folder_path: &str) {
        let path = Path::new(folder_path);
        Self::visit_dirs(path, &Self::send_file);
    }

    fn send_file(dir_entry: &DirEntry) {
        let mut stream = TcpStream::connect("10.0.0.139:8888").unwrap();

        // First write Folderpath
        match stream.write_all(dir_entry.path().to_string_lossy().as_bytes()) {
            Ok(_) => {}
            Err(_) => {
                print!("Error while writing folder name");
                return;
            }
        }

        let mut client_buffer = [0u8; 1024];

        loop {
            match stream.read(&mut client_buffer) {
                Ok(n) => {
                    if n == 0 {
                        continue;
                    } else {
                        let server_resp = String::from_utf8(client_buffer.to_vec());
                        print!("{}", server_resp.unwrap());
                        break;
                    }
                }
                Err(error) => print!("{}", error),
            }
        }
        //
    }

    fn visit_dirs(dir: &Path, cb: &dyn Fn(&DirEntry)) -> io::Result<()> {
        if dir.is_dir() {
            print!("Is dir true for {}", dir.to_str().unwrap());
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();

                if path.is_dir() {
                    Self::visit_dirs(&path, cb)?;
                } else {
                    cb(&entry);
                }
            }
        } else {
            print!("Is dir true for {}", dir.to_str().unwrap());
        }

        Ok(())
    }
}
