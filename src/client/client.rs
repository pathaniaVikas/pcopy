use std::{
    fs::File,
    io::{Read, Write},
    net::TcpStream,
    os::unix::ffi::OsStrExt,
    path::Path,
};

use crc32fast::Hasher;
use tracing::{debug, error};
use walkdir::WalkDir;

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

    fn server_address(&self) -> String {
        self.server_ip.clone() + ":" + self.server_port.to_string().as_str()
    }

    pub fn send_folder(&mut self, folder_path: &Path) {
        for entry in WalkDir::new(folder_path)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| !e.file_type().is_dir())
        {
            self.send_file(entry.path());
        }
    }

    pub fn send_file(&mut self, file_path: &Path) {
        let mut stream = TcpStream::connect(self.server_address()).unwrap();

        let mut file = File::open(file_path).unwrap();

        // Send Frame
        // |Operation|file_path_length_bytes|file_path|checksum|file_data_length_bytes|file_data|checksum|
        // Operation 02 for create_file
        let mut buf = vec![0b00000010u8];
        let file_name_bytes = file_path.as_os_str().as_bytes().to_vec();
        buf.extend_from_slice(&Self::get_length_as_u16_bytes(file_name_bytes.clone()).to_vec());
        buf.extend_from_slice(&file_name_bytes.clone());
        buf.extend_from_slice(
            &Self::get_checksum_bytes(file_name_bytes.clone())
                .to_be_bytes()
                .to_vec(),
        );

        // Add |file_data_bytes_length|file_data|checksum|

        // Currently server is designed with 32 bit length bytes, lets fit it into u32
        let file_data_len = match u32::try_from(file.metadata().unwrap().len()) {
            Ok(x) => x,
            Err(e) => {
                error!(
                    "File size {} is more than supported size {}. Got error while fitting into u32: {}",
                    file.metadata().unwrap().len(),
                    u32::MAX,
                    e
                );
                return;
            }
        };

        debug!("File data length bytes {}", file_data_len);
        buf.extend_from_slice(&file_data_len.to_be_bytes().to_vec());

        let mut current_read_index = buf.len();
        let mut file_bytes_read_total = 0;
        // expand buf to be of 1024 length
        buf.resize(1024, 0b00000000u8);
        assert!(buf.len() == 1024);

        // Fill remaining bytes in buffer with file data
        let n = file.read(&mut buf[current_read_index..]).unwrap();
        // initiate checksum computer
        let mut file_checksum_hasher = Hasher::new();
        file_checksum_hasher.update(&buf[current_read_index..current_read_index + n]);
        file_bytes_read_total += n;
        current_read_index += n;

        match stream.write_all(&buf[0..current_read_index]) {
            Ok(_) => debug!("First request buffer written to server"),
            Err(e) => debug!("Error while writing first buffer {}", e),
        };

        loop {
            // debug!("Before buffer length {}", buf.len());
            // Read from file and fill buffer's 1024 bytes again
            let n = file.read(&mut buf[0..]).unwrap();
            // debug!("After buffer length {}", buf.len());
            if n == 0 {
                break;
            }
            stream.write_all(&buf[0..n]);
            debug!("{n} bytes written to stream");
            stream.flush().unwrap();
            file_checksum_hasher.update(&buf[0..n]);
            file_bytes_read_total += n;
        }

        debug!("Total file bytes read {file_bytes_read_total}");
        // write checksum
        let checksum = file_checksum_hasher.finalize();
        debug!("Checksum {:?}", checksum.to_be_bytes());
        let _ = stream.write_all(&checksum.to_be_bytes());
        stream.flush().unwrap();
        // Check server status of file writing
        // let mut result_buff = [0b00000000u8; 2];
        // stream.read(&mut result_buff);

        // if i16::from_be_bytes(result_buff) == -1 {
        //     error!(
        //         "File({:#?}) failed to copy to server ",
        //         file_path.as_os_str()
        //     );
        // } else {
        //     info!(
        //         "File({:#?}) succesfully copied to server",
        //         file_path.as_os_str()
        //     )
        // }
    }

    // fn visit_dirs(dir: &Path, cb: &dyn Fn(&DirEntry)) -> io::Result<()> {
    //     if dir.is_dir() {
    //         print!("Is dir true for {}", dir.to_str().unwrap());
    //         for entry in fs::read_dir(dir)? {
    //             let entry = entry?;
    //             let path = entry.path();

    //             if path.is_dir() {
    //                 Self::visit_dirs(&path, cb)?;
    //             } else {
    //                 cb(&entry);
    //             }
    //         }
    //     } else {
    //         print!("Is dir true for {}", dir.to_str().unwrap());
    //     }

    //     Ok(())
    // }

    fn get_length_as_u16_bytes(path: Vec<u8>) -> [u8; 2] {
        u16::try_from(path.clone().len()).unwrap().to_be_bytes()
    }

    fn get_checksum_bytes(buf: Vec<u8>) -> u32 {
        let mut hasher = Hasher::new();
        hasher.update(&buf);
        hasher.finalize()
    }

    fn get_length_as_u32_bytes(path: Vec<u8>) -> [u8; 4] {
        u32::try_from(path.clone().len()).unwrap().to_be_bytes()
    }
}
