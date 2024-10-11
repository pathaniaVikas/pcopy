use std::{
    fs::{self},
    io::{self, Error, ErrorKind, Read},
    path::Path,
};

use byteorder::{BigEndian, ByteOrder};
use crc32fast::Hasher;
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
};
use tracing::{debug, error, info};

static ROOT_SAVING_DIRECTORY: &str = "/Users/vikaspathania/Downloads/Backup";

/// Creating this for unit tests
pub trait ReadFromStream {
    // Using Vec<u8> for help in unit testing
    async fn read_data(
        &mut self,
        buf: &mut Vec<u8>,
        valid_read_index: usize,
    ) -> Result<usize, std::io::Error>;
}

impl ReadFromStream for TcpStream {
    async fn read_data(
        &mut self,
        buf: &mut Vec<u8>,
        valid_read_index: usize,
    ) -> Result<usize, std::io::Error> {
        // We need to append data after the valid_read_index.
        self.read(&mut buf[valid_read_index..]).await
    }
}

enum Operation {
    // 01
    CreateFolder = 0x01,
    // 02
    SaveFile = 0x02,
}

pub struct Server {
    port: u32,
    ip: String,
}

impl Server {
    pub fn init(ip: String, port: u32) -> Self {
        Self { ip, port }
    }

    ///
    /// |---------------------------------------------------|
    /// |operation|  length   |   data         | checksum   |
    /// | 1 byte  |  2 bytes |   length bytes  | 32 bit     |
    /// |---------------------------------------------------|
    ///
    /// - Read 1st operation Byte: Already Done before this method
    /// - Read  2 bytes for length of data
    /// - Read length bytes for actual folder_name utf8 bytes.
    /// - Read 32 bit checksum
    /// - compute and compare checksum
    /// - create folder
    ///
    pub async fn create_folder<T: ReadFromStream>(
        mut socket: T,
        buf: &mut Vec<u8>,
        // Usually buf.len() if buf is of type vector. But if buf is of type fixed array, then buf.len() will
        // give size of array, whereas this will tell the index of last readable byte. Bytes after this will be garbage
        mut valid_read_index: usize,
        // Index into buffer, for how many bytes we have already processed
        mut current_processed_index: usize,
    ) -> Result<(usize, usize), Error> {
        debug!(
            "Recived buf of length {}, with number of valid bytes till {} ",
            buf.len(),
            valid_read_index
        );
        let data_length_bytes: usize = 2;

        // Read data_length from buffer which should be next `data_length_bytes`[2] bytes from current_processed_index
        // First check, if we have `data_length_bytes` available in data read from stream
        // If not, loop over stream, until we get >= `data_length_bytes`
        loop {
            if (valid_read_index - current_processed_index) < data_length_bytes {
                // Read more bytes
                let n = socket
                    .read_data(buf, valid_read_index)
                    .await
                    .expect("failed to read data from socket");
                if n <= 0 {
                    // We have no data from stream, return error
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        "No data to read from stream. Cannot create Folder",
                    ));
                }
                valid_read_index += n;
            } else {
                break;
            }
        }

        let folder_name_length = BigEndian::read_u16(
            &buf[current_processed_index..current_processed_index + data_length_bytes],
        );
        // update current_bytes_processed_index to include data_length_bytes
        current_processed_index += data_length_bytes;

        debug!(
            "Current bytes processed after reading folder name length bytes {} and toal bytes read from stream {}",
            current_processed_index,
            valid_read_index
        );

        debug!("Folder Name has {} bytes", folder_name_length as usize);

        // Next read the folder_name_length bytes from stream, if not already read.
        loop {
            if (valid_read_index - current_processed_index) < folder_name_length as usize {
                // Read more bytes
                let n = socket
                    .read_data(buf, valid_read_index)
                    .await
                    .expect("failed to read data from socket");
                if n <= 0 {
                    // We have no data from stream, return error
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        format!(
                            "Cannot read full folder name bytes. No of folder name bytes read {}",
                            valid_read_index - current_processed_index
                        ),
                    ));
                }
                valid_read_index += n;
            } else {
                break;
            }
        }

        let relative_folder_path = match String::from_utf8(
            buf[current_processed_index..current_processed_index + folder_name_length as usize]
                .to_vec(),
        ) {
            Ok(rp) => rp,
            Err(e) => {
                debug!(
                    "Error while converting foldername bytes to utf8 string. \n Error: {:#?}",
                    e
                );
                return Err(Error::new(
                    ErrorKind::Other,
                    format!(
                        "Error converting bytes to utf8 for foldername for bytes {:#?}",
                        buf[current_processed_index
                            ..current_processed_index + folder_name_length as usize]
                            .to_vec()
                    ),
                ));
            }
        };

        // update processed bytes index to include folder_name length.
        current_processed_index += folder_name_length as usize;

        debug!(
            "Current bytes processed after reading folder name {} and toal bytes read from stream {}",
            current_processed_index,
            valid_read_index
        );

        // Read 32 bits checksum - 4 bytes
        let checsum_bytes_length = 4;

        loop {
            if valid_read_index - current_processed_index < checsum_bytes_length {
                let n = socket
                    .read_data(buf, valid_read_index)
                    .await
                    .expect("failed to read data from socket");
                if n <= 0 {
                    // We have no data from stream, return error
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        format!(
                            "Cannot read checksum from stream. No of checksum bytes read {}",
                            valid_read_index - current_processed_index
                        ),
                    ));
                }
                valid_read_index += n;
            } else {
                break;
            }
        }

        // Read checksum
        let received_checksum = Hasher::new_with_initial(BigEndian::read_u32(
            &buf[current_processed_index..current_processed_index + checsum_bytes_length],
        ))
        .finalize();

        // update current bytes processed to include checksum
        current_processed_index += checsum_bytes_length;

        let mut hasher = Hasher::new();
        hasher.update(relative_folder_path.as_bytes());
        let expected_checkeum = hasher.finalize();

        // If checksum matches, create the folder, else log error and move on
        if expected_checkeum == received_checksum {
            // if relative folder path starts with "/", remove it, or else Path::join will do replace. Read join documentation
            let complete_folder_path = Path::new(ROOT_SAVING_DIRECTORY).join(
                relative_folder_path
                    .strip_prefix("/")
                    .unwrap_or(&relative_folder_path),
            );
            debug!("Folder path to be created {:#?} ", complete_folder_path);
            match fs::create_dir_all(complete_folder_path.clone()) {
                Ok(_) => debug!("Folder created succesfully {:#?}", complete_folder_path),
                Err(e) => error!(
                    "Error creating folder{:#?}.\n## Error ##\n{}",
                    complete_folder_path, e
                ),
            }
        } else {
            error!("Recieved checksum {} and expected checksum {} did not match. Skipping folder creation"
            , received_checksum, expected_checkeum);
        }

        debug!(
            "Current bytes processed after reading checksum {} and toal bytes read from stream {}",
            current_processed_index, valid_read_index
        );

        Ok((current_processed_index, valid_read_index))
    }

    // pub fn save_file(socket: &TcpStream, buf: &Vec<u8>) -> io::Result<()> {}

    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        let addr = self.server_address();
        let listener = TcpListener::bind(&addr).await?;

        info!("Server started on ip: {}", addr);

        loop {
            // Asynchronously wait for an inbound socket.
            let (mut socket, _) = listener.accept().await?;

            debug!("Received request");
            tokio::spawn(async move {
                let mut buf = vec![0; 1024];

                let n = socket
                    .read(&mut buf)
                    .await
                    .expect("failed to read data from socket");

                // Connection created but no data, end the request.
                if n <= 0 {
                    error!("{n} bytes found for request, closing connection");
                    return;
                }

                // First byte is [`Operation`] byte either 01 or 02
                if buf[0] & Operation::CreateFolder as u8 == 1 {
                    match Self::create_folder(socket, &mut buf, n, 1).await {
                        Ok(_) => debug!("Folder created sucesfully"),
                        Err(_) => error!("Folder failed to create, read debug logs"),
                    }
                } else if buf[0] == Operation::SaveFile as u8 {
                    // save_file(socket, buf);
                    error!("Not Implemented");
                    return;
                } else {
                    error!("Unknown operation {:?}. closing connection", buf[0]);
                    return;
                }
            });
        }
    }

    fn server_address(&self) -> String {
        self.ip.clone() + ":" + self.port.to_string().as_str()
    }
}

#[cfg(test)]
mod tests {

    use crc32fast::Hasher;
    use std::sync::Once;
    use tracing::debug;

    use super::{ReadFromStream, Server};

    static INIT: Once = Once::new();

    pub fn initialize() {
        INIT.call_once(|| {
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::DEBUG)
                .with_target(false)
                .init();
        });
    }
    struct TcpStreamTestStreamNoData {}

    impl ReadFromStream for TcpStreamTestStreamNoData {
        async fn read_data(
            &mut self,
            buf: &mut Vec<u8>,
            valid_read_index: usize,
        ) -> Result<usize, std::io::Error> {
            Ok(0)
        }
    }

    struct TcpStreamTestStreamAllData {}

    impl ReadFromStream for TcpStreamTestStreamAllData {
        // This will fill buffer with folder_name and its metadata
        async fn read_data(
            &mut self,
            buf: &mut Vec<u8>,
            valid_read_index: usize,
        ) -> Result<usize, std::io::Error> {
            let initial_buf_len = buf.len();
            let folder_name_bytes = get_folder_name_bytes();
            buf.extend_from_slice(&get_folder_length_bytes(folder_name_bytes.clone()));
            buf.extend_from_slice(&folder_name_bytes.clone());
            buf.extend_from_slice(&get_checksum_bytes(folder_name_bytes));

            debug!(
                "folder data buffer {}",
                String::from_utf8(buf.to_vec()).unwrap()
            );
            debug!("{:#?}", buf.len());
            Ok(buf.len() - initial_buf_len) // Return no of new bytes added
        }
    }

    struct TcpStreamTestStreamWriteFolderNameAndChecksum {}

    impl ReadFromStream for TcpStreamTestStreamWriteFolderNameAndChecksum {
        async fn read_data(
            &mut self,
            buf: &mut Vec<u8>,
            valid_read_index: usize,
        ) -> Result<usize, std::io::Error> {
            let initial_buf_len = buf.len();
            let folder_name_bytes = get_folder_name_bytes();
            buf.extend_from_slice(&folder_name_bytes);
            buf.extend_from_slice(&get_checksum_bytes(folder_name_bytes));
            Ok(buf.len() - initial_buf_len)
        }
    }

    struct TcpStreamTestStreamWriteChecksum {}

    impl ReadFromStream for TcpStreamTestStreamWriteChecksum {
        async fn read_data(
            &mut self,
            buf: &mut Vec<u8>,
            valid_read_index: usize,
        ) -> Result<usize, std::io::Error> {
            let initial_buf_len = buf.len();
            let folder_name_bytes = get_folder_name_bytes();
            buf.extend_from_slice(&get_checksum_bytes(folder_name_bytes));
            Ok(buf.len() - initial_buf_len)
        }
    }

    fn get_folder_length_bytes(folder_path: Vec<u8>) -> Vec<u8> {
        u16::try_from(folder_path.clone().len())
            .unwrap()
            .to_be_bytes()
            .to_vec()
    }

    fn get_checksum_bytes(buf: Vec<u8>) -> Vec<u8> {
        let mut hasher = Hasher::new();
        hasher.update(&buf);
        hasher.finalize().to_be_bytes().to_vec()
    }

    fn get_folder_name_bytes() -> Vec<u8> {
        "/This/is/the/test/folderऐक".as_bytes().to_vec()
    }

    #[tokio::test]
    async fn test_create_folder_ok_read_data_from_stream() {
        initialize();
        let mut test_stream = TcpStreamTestStreamAllData {};

        let mut buf = vec![0b00000001u8];

        let folder_name_bytes = get_folder_name_bytes();
        buf.extend_from_slice(&get_folder_length_bytes(folder_name_bytes.clone()));
        buf.extend_from_slice(&folder_name_bytes.clone());
        buf.extend_from_slice(&get_checksum_bytes(folder_name_bytes));

        let number_of_bytes_read = buf.len();
        debug!("Buf created for test {:#?}", String::from_utf8(buf.clone()));

        match Server::create_folder(test_stream, &mut buf, number_of_bytes_read, 1).await {
            Ok((bytes_processed, bytes_read)) => {
                // Assert bytes processed returned is equal to number of bytes in buffer happy case
                assert_eq!(bytes_processed, number_of_bytes_read);
                // Assert bytes read during operation is equal to number of bytes in buffer
                // This will also assert we never called TcpStreamTestGood::read_data method, as we passed full buffer.
                assert_eq!(bytes_read, number_of_bytes_read);
            }

            Err(e) => {
                print!("{}", e);
                panic!()
            }
        }
    }

    #[tokio::test]
    async fn test_create_folder_ok_no_read_from_stream() {
        initialize();

        let mut test_stream = TcpStreamTestStreamAllData {};

        let mut buf = vec![0b00000001u8];
        let number_of_bytes_read = buf.len();
        match Server::create_folder(test_stream, &mut buf, number_of_bytes_read, 1).await {
            Ok((bytes_processed, bytes_read)) => {
                // Assert bytes processed returned is equal to number of bytes in buffer happy case
                assert_eq!(bytes_processed, buf.len());
                // Assert bytes read during operation is equal to number of bytes in buffer
                // This will also assert we never called TcpStreamTestGood::read_data method, as we passed full buffer.
                assert_eq!(bytes_read, buf.len());
            }

            Err(e) => {
                print!("{}", e);
                panic!()
            }
        }
    }

    #[tokio::test]
    async fn test_create_folder_ok_read_folder_name_and_checksum_from_stream() {
        initialize();

        let mut test_stream = TcpStreamTestStreamWriteFolderNameAndChecksum {};

        let mut buf = vec![0b00000001u8];
        let folder_name_bytes = get_folder_name_bytes();
        buf.extend_from_slice(&get_folder_length_bytes(folder_name_bytes.clone()));

        let number_of_bytes_read = buf.len();
        match Server::create_folder(test_stream, &mut buf, number_of_bytes_read, 1).await {
            Ok((bytes_processed, bytes_read)) => {
                // Assert bytes processed returned is equal to number of bytes in buffer happy case
                assert_eq!(bytes_processed, buf.len());
                // Assert bytes read during operation is equal to number of bytes in buffer
                // This will also assert we never called TcpStreamTestGood::read_data method, as we passed full buffer.
                assert_eq!(bytes_read, buf.len());
            }

            Err(e) => {
                print!("{}", e);
                panic!()
            }
        }
    }

    #[tokio::test]
    async fn test_create_folder_ok_read_checksum_from_stream() {
        initialize();

        let mut test_stream = TcpStreamTestStreamWriteChecksum {};

        let mut buf = vec![0b00000001u8];
        let folder_name_bytes = get_folder_name_bytes();
        buf.extend_from_slice(&get_folder_length_bytes(folder_name_bytes.clone()));
        buf.extend_from_slice(&folder_name_bytes.clone());

        let number_of_bytes_read = buf.len();
        match Server::create_folder(test_stream, &mut buf, number_of_bytes_read, 1).await {
            Ok((bytes_processed, bytes_read)) => {
                // Assert bytes processed returned is equal to number of bytes in buffer happy case
                assert_eq!(bytes_processed, buf.len());
                // Assert bytes read during operation is equal to number of bytes in buffer
                // This will also assert we never called TcpStreamTestGood::read_data method, as we passed full buffer.
                assert_eq!(bytes_read, buf.len());
            }

            Err(e) => {
                print!("{}", e);
                panic!()
            }
        }
    }

    #[tokio::test]
    async fn test_create_folder_erro_read_stream() {
        initialize();

        let test_stream = TcpStreamTestStreamNoData {};

        let mut buf = vec![0b00000001u8];
        let number_of_bytes_read = buf.len();
        match Server::create_folder(test_stream, &mut buf, number_of_bytes_read, 1).await {
            Ok((bytes_processed, bytes_read)) => {
                // Assert bytes processed returned is equal to number of bytes in buffer happy case
                assert_eq!(bytes_processed, buf.len());
                // Assert bytes read during operation is equal to number of bytes in buffer
                // This will also assert we never called TcpStreamTestGood::read_data method, as we passed full buffer.
                assert_eq!(bytes_read, buf.len());
            }

            Err(e) => {
                print!("{}", e);
            }
        }
    }
}
