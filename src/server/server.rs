use std::{
    cmp::min,
    fs::{self, File},
    io::{Error, ErrorKind, Write},
    os::unix::ffi::OsStrExt,
    path::Path,
};

use byteorder::{BigEndian, ByteOrder};
use crc32fast::Hasher;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
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
        // Write data into provided buffer from start_index
        start_index: usize,
    ) -> Result<usize, std::io::Error>;
}

impl ReadFromStream for TcpStream {
    async fn read_data(
        &mut self,
        buf: &mut Vec<u8>,
        start_index: usize,
    ) -> Result<usize, std::io::Error> {
        // We need to append data after the start_index.
        self.read(&mut buf[start_index..]).await
    }
}

enum Operation {
    // 01
    CreateParentFolder = 0x01,
    // 02
    SaveFile = 0x02,
}

struct BufferState {
    start_index: usize,
    end_index: usize,
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
    /// |------------------------------------------|
    /// |  length   |   data          | checksum   |
    /// |  2 bytes  |   length bytes  | 32 bit     |
    /// |------------------------------------------|
    ///
    /// - Read 2 bytes for length of data
    /// - Read length bytes for actual folder_name utf8 bytes.
    /// - Read 32 bit checksum
    /// - compute and compare checksum
    /// - create Parent Folder (eg. if path is "/a/b/c.txt", we will create "/a/b")
    ///
    /// ATTENTION:
    /// Given a path like "This/is/a/test/foo/bar"
    /// This method will create parent folder "This/is/a/test/foo" skipping "bar" folder.
    /// But it will return the whole path "This/is/a/test/foo/bar"
    /// It is the duty of caller to create "bar"
    async fn create_parent_folder<T: ReadFromStream>(
        socket: &mut T,
        buf: &mut Vec<u8>,
        // Index into buffer, for how many bytes are valid (read from socket)
        mut end_index: usize,
        // Index into buffer, for how many bytes we have already processed
        mut start_index: usize,
    ) -> Result<(BufferState, String), Error> {
        debug!(
            "Recived buf of length {}, with number of valid bytes from start: {} to end: {} ",
            buf.len(),
            start_index,
            end_index
        );

        let data_length_bytes: usize = 2;

        // Read data_length from buffer which should be next `data_length_bytes` ie 2 bytes from start_index
        // First check, if we have `data_length_bytes` available in data read from stream
        // If not, loop over stream, until we get >= `data_length_bytes` from stream
        loop {
            if (end_index - start_index) < data_length_bytes {
                // Read more bytes
                let n = socket
                    .read_data(buf, end_index)
                    .await
                    .expect("failed to read data from socket");
                if n <= 0 {
                    // We have no data from stream, return error
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        "No data to read from stream. Cannot create Folder",
                    ));
                }
                end_index += n;
            } else {
                break;
            }
        }

        let file_path = BigEndian::read_u16(&buf[start_index..start_index + data_length_bytes]);
        start_index += data_length_bytes;

        debug!(
            "Buffer: Start index after reading File path lenght bytes(2) {} and total bytes available to process {}",
            start_index,
            end_index-start_index
        );

        debug!("File path has length: {} bytes", file_path as usize);

        // Next read the actual file_path bytes from stream, if not already read.
        loop {
            if (end_index - start_index) < file_path as usize {
                // Read more bytes
                let n = socket
                    .read_data(buf, end_index)
                    .await
                    .expect("failed to read data from socket");
                if n <= 0 {
                    // We have no data from stream, return error
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        format!(
                            "Cannot read full file path bytes. No of file path bytes read {}",
                            end_index - start_index
                        ),
                    ));
                }
                end_index += n;
            } else {
                break;
            }
        }

        let relative_file_path =
            match String::from_utf8(buf[start_index..start_index + file_path as usize].to_vec()) {
                Ok(rp) => rp,
                Err(e) => {
                    debug!(
                        "Error while converting file path bytes to utf8 string. \n Error: {:#?}",
                        e
                    );
                    return Err(Error::new(
                        ErrorKind::Other,
                        format!(
                            "Error converting file path bytes to utf8 {:#?}",
                            buf[start_index..start_index + file_path as usize].to_vec()
                        ),
                    ));
                }
            };

        start_index += file_path as usize;

        debug!(
            "Start index {} , End index {} after reading Folder name",
            start_index, end_index
        );

        let received_checksum: u32 =
            match read_checksum(&mut end_index, &mut start_index, socket, buf).await {
                Ok(value) => value,
                Err(e) => return Err(e),
            };

        let mut hasher = Hasher::new();
        hasher.update(relative_file_path.as_bytes());
        let expected_checkeum = hasher.finalize();

        // Prefix with download directory
        // if relative File path starts with "/", remove it, or else Path::join will do replace. Read join documentation
        let complete_file_path = Path::new(ROOT_SAVING_DIRECTORY).join(
            relative_file_path
                .strip_prefix("/")
                .unwrap_or(&relative_file_path),
        );

        // If checksum matches, create the Parent folder, else throw error
        if expected_checkeum == received_checksum {
            debug!("Complete file path {:#?} ", complete_file_path);
            // Just create parent directory, file should be created by caller
            let parent_folder_path = if complete_file_path.ends_with(Path::new("/")) {
                complete_file_path.parent().unwrap().parent().unwrap()
            } else {
                complete_file_path.parent().unwrap()
            };

            debug!(
                "Creating Parent Folder {}",
                parent_folder_path.to_str().unwrap()
            );

            match fs::create_dir_all(parent_folder_path) {
                Ok(_) => debug!("Folder created succesfully {:#?}", parent_folder_path),
                Err(e) => {
                    error!(
                        "Error creating Folder{:#?}.\n## Error ##\n{}",
                        parent_folder_path, e
                    );

                    return Err(Error::new(
                        ErrorKind::Other,
                        format!("Error creating parent folder {e}"),
                    ));
                }
            }
        } else {
            error!("Recieved checksum {} and expected checksum {} did not match. Skipping Parent Folder creation"
            , received_checksum, expected_checkeum);

            return Err(Error::new(
                ErrorKind::Other,
                format!(
                    "Error creating parent folder, checksum mismatched received: {}, expected {}",
                    received_checksum, expected_checkeum
                ),
            ));
        }

        debug!(
            "Start Index {} and End Index {}, after completing parent folder creation",
            start_index, end_index
        );

        Ok((
            BufferState {
                start_index,
                end_index,
            },
            String::from_utf8(complete_file_path.as_os_str().as_bytes().to_vec()).unwrap(),
        ))
    }

    ///
    ///  --------------------------------------------------------------------------------------------------------------------
    /// | file_path_length |  file_path_bytes          | checksum | file_data_length | file_data_bytes       | file_checksum |
    /// |    2 bytes       |  file_path_length_bytes   | 4 bytes  |  4 bytes         | file-data-length_bytes| 4 bytes       |
    ///  --------------------------------------------------------------------------------------------------------------------
    ///
    async fn save_file<T: ReadFromStream>(
        socket: &mut T,
        buf: &mut Vec<u8>,
        // Index into buffer, for how many bytes are valid bytes to process
        mut end_index: usize,
        // Index into buffer, for how many bytes we have already processed
        mut start_index: usize,
    ) -> Result<(BufferState, String), Error> {
        debug!(
            "Recived buf of length {}, with number of valid bytes to process {} ",
            buf.len(),
            end_index - start_index
        );

        let mut file: Option<File> = Option::None;
        let mut complete_file_path = Option::None;

        match Self::create_parent_folder(socket, buf, end_index, start_index).await {
            Ok((buffer_state, fp)) => {
                end_index = buffer_state.end_index;
                start_index = buffer_state.start_index;

                debug!("Creating file at path: {}", fp);
                file = Some(File::create(fp.clone())?);
                complete_file_path = Some(fp);
            }
            Err(e) => return Err(Error::new(ErrorKind::InvalidInput, e)),
        }

        let mut file = file.unwrap();

        debug!(
            "StartIndex {} and EndIndex {} after reading file_path",
            start_index, end_index
        );

        // Next read actual file data
        let data_length_bytes: usize = 4;

        // Read data_length from buffer which should be next `data_length_bytes`[4] bytes from start_index
        // First check, if we have `data_length_bytes` available in data read from stream
        // If not, loop over stream, until we get >= `data_length_bytes`
        loop {
            if (end_index - start_index) < data_length_bytes {
                // Read more bytes
                let n = socket
                    .read_data(buf, end_index)
                    .await
                    .expect("failed to read data from socket");
                if n <= 0 {
                    // We have no data from stream, return error
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        "No data to read from stream. Cannot read file data length",
                    ));
                }
                end_index += n;
            } else {
                break;
            }
        }

        let mut file_data_length =
            BigEndian::read_u32(&buf[start_index..start_index + data_length_bytes]);

        start_index += data_length_bytes;

        debug!(
            "StartIndex {} and EndIndex {} after reading file data length bytes",
            start_index, end_index
        );

        debug!("File will have {} bytes", file_data_length);

        let mut expected_checksum_hasher = Hasher::new();

        // write remaining bytes in buffer to file
        // TODO: It will panic if errored during write. Need to read result and return Err
        // Also Init `total_file_data_bytes_read` to log number of file bytes read from stream for debugging

        // If file is small and can fit in < 1023 bytes, we need to read till that index
        let file_data_end_index = min(start_index + file_data_length as usize, end_index);

        debug!("Reading first buffer data from {start_index} to {file_data_end_index}");
        let mut total_file_data_bytes_read =
            file.write(&buf[start_index..file_data_end_index]).unwrap();
        debug!("Total bytes read from initial buffer {total_file_data_bytes_read}");

        expected_checksum_hasher.update(&buf[start_index..file_data_end_index]);

        file_data_length -= total_file_data_bytes_read as u32;

        start_index = file_data_end_index;

        debug!(
            "StartIndex {start_index}, EndIndex {end_index}, FileDataRemaining {file_data_length}"
        );

        loop {
            if (end_index - start_index) < file_data_length as usize {
                // IF we have read all data in buffer, reset indexes
                if start_index == buf.len() {
                    // Now we can reset our pointers to use whole buffer for reading data from stream
                    end_index = 0;
                    start_index = 0;
                }

                // Read file data bytes and write to file
                let n = socket.read_data(buf, end_index).await.expect(&format!(
                    "failed to read data from socket, cpi: {}, vri: {}, fdl: {}",
                    start_index, end_index, file_data_length
                ));
                if n <= 0 {
                    // We have no data from stream, return error
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        format!(
                            "Cannot read full File name bytes. No of File name bytes read {}",
                            end_index - start_index
                        ),
                    ));
                }

                end_index += n;
                // total_file_data_bytes_read += n;
                // file_data_to_read -= n as u32;

                // if we have read more than remaining file length bytes from stream.
                // It means we have read bytes of checksum too in same buffer
                // provided after the file.
                // Eg.
                //                                  |xxxxxxxxxxxxxxx|ccccccccccccc|
                //                                  |file_data_bytes|checksum bits|
                // current_processed_index is here ->                              <- valid_read_index is here
                // Then write only file data bytes to file
                if end_index > start_index + file_data_length as usize {
                    debug!("Writing last file data chunk. CurrentProcessIndex {start_index}, ValidReadIndex {end_index}, FileDataLengthRemainig {file_data_length}, TotalBytesRead {total_file_data_bytes_read}");
                    let current_data_slice =
                        &buf[start_index..start_index + file_data_length as usize];

                    file.write_all(current_data_slice)?;
                    // file.flush().unwrap();
                    expected_checksum_hasher.update(current_data_slice);

                    start_index += file_data_length as usize;

                    total_file_data_bytes_read += file_data_length as usize;
                    // TODO: Should we use u32 instead of usize
                    file_data_length -= start_index as u32;
                    // File data length should be zero here
                    assert!(file_data_length == 0);
                } else {
                    let current_data_slice = &buf[start_index..end_index];

                    // TODO: It can panic here: read result and return error
                    file.write_all(current_data_slice).unwrap();
                    // file.flush().unwrap(); // TODO do we need to flush everytime ?
                    expected_checksum_hasher.update(current_data_slice);

                    start_index += n;

                    total_file_data_bytes_read += n;
                    file_data_length -= n as u32;
                }

                // debug!("cpi {current_processed_index}, vri {valid_read_index}, n {n}, tbr {total_file_data_bytes_read}");

                // debug!("File bytes read {}", total_file_data_bytes_read);
            } else {
                debug!(
                    "Data read completely. Total file bytes received {}",
                    total_file_data_bytes_read
                );
                break;
            }
        }

        // Read 32 bits checksum
        let received_checksum: u32 =
            match read_checksum(&mut end_index, &mut start_index, socket, buf).await {
                Ok(value) => value,
                Err(e) => return Err(e),
            };

        let expected_checksum = expected_checksum_hasher.finalize();
        if received_checksum != expected_checksum {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!(
                    "Computed checksum {} and received checksum {} do not match for file {}",
                    expected_checksum,
                    received_checksum,
                    complete_file_path.unwrap()
                ),
            ));
        }

        Ok((
            BufferState {
                start_index,
                end_index,
            },
            complete_file_path.unwrap(),
        ))
    }

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
                    error!("{n} bytes found for request. Closing connection");
                    let result: i16 = -1;
                    match socket.write_all(&result.to_be_bytes()).await {
                        Ok(_) => {}
                        Err(e) => error!("Error writing response to client {e}"),
                    }
                    return;
                }

                // First byte is [`Operation`] byte, either 000000001 or 000000010
                if buf[0] == Operation::CreateParentFolder as u8 {
                    match Self::create_parent_folder(&mut socket, &mut buf, n, 1).await {
                        Ok(_) => debug!("Folder created sucesfully"),
                        Err(e) => {
                            error!("File failed to create with error\n{e}, read debug logs. Closing connection");
                            let result: i16 = -1;
                            socket.write_all(&result.to_be_bytes());
                            return;
                        }
                    }
                } else if buf[0] == Operation::SaveFile as u8 {
                    match Self::save_file(&mut socket, &mut buf, n, 1).await {
                        Ok((buffer_state, file_path)) => {
                            debug!("File saved succesfully at {}", file_path);
                        }
                        Err(e) => {
                            error!("File failed to save with error\n{e}, read debug logs. Closing connection");
                            let result: i16 = -1;
                            socket.write_all(&result.to_be_bytes()).await;
                            return;
                        }
                    }
                } else {
                    error!("Unknown operation {:?}. Closing connection", buf[0]);
                    return;
                }

                // 1 is OK, -1 is ERROR
                let result: u16 = 1;
                socket.write_all(&result.to_be_bytes()).await;
                debug!("Request completed");
            });
        }
    }

    fn server_address(&self) -> String {
        self.ip.clone() + ":" + self.port.to_string().as_str()
    }
}

async fn read_checksum<T: ReadFromStream>(
    end_index: &mut usize,
    start_index: &mut usize,
    socket: &mut T,
    buf: &mut Vec<u8>,
) -> Result<u32, Error> {
    let checksum_bytes_length = 4;
    loop {
        if *end_index - *start_index < checksum_bytes_length {
            let n = socket
                .read_data(buf, *end_index)
                .await
                .expect("failed to read data from socket");
            if n <= 0 {
                // We have no data from stream, return error
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!(
                        "Cannot read checksum from stream. No of checksum bytes read {}",
                        *end_index - *start_index
                    ),
                ));
            }
            *end_index += n;
        } else {
            break;
        }
    }

    debug!(
        "Checksum bytes: {:#?} from {start_index}",
        &buf[*start_index..*start_index + checksum_bytes_length]
    );
    let received_checksum = Hasher::new_with_initial(BigEndian::read_u32(
        &buf[*start_index..*start_index + checksum_bytes_length],
    ))
    .finalize();

    // update current bytes processed to include checksum
    *start_index += checksum_bytes_length;

    Ok(received_checksum)
}

#[cfg(test)]
mod tests {

    use crc32fast::Hasher;
    use std::{fs::File, io::Write, sync::Once};
    use tracing::debug;

    use crate::server::server::ROOT_SAVING_DIRECTORY;

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
            _buf: &mut Vec<u8>,
            _valid_read_index: usize,
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
            _valid_read_index: usize,
        ) -> Result<usize, std::io::Error> {
            let initial_buf_len = buf.len();
            let folder_name_bytes = get_folder_name().as_bytes().to_vec();
            buf.extend_from_slice(&get_length_as_u16_bytes(folder_name_bytes.clone()));
            buf.extend_from_slice(&folder_name_bytes.clone());
            buf.extend_from_slice(&get_checksum_bytes(folder_name_bytes).to_be_bytes().to_vec());

            debug!(
                "File data buffer {}",
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
            _valid_read_index: usize,
        ) -> Result<usize, std::io::Error> {
            let initial_buf_len = buf.len();
            let folder_name_bytes = get_folder_name().as_bytes().to_vec();
            buf.extend_from_slice(&folder_name_bytes);
            buf.extend_from_slice(&get_checksum_bytes(folder_name_bytes).to_be_bytes().to_vec());
            Ok(buf.len() - initial_buf_len)
        }
    }

    struct TcpStreamTestStreamWriteChecksum {}

    impl ReadFromStream for TcpStreamTestStreamWriteChecksum {
        async fn read_data(
            &mut self,
            buf: &mut Vec<u8>,
            _valid_read_index: usize,
        ) -> Result<usize, std::io::Error> {
            let initial_buf_len = buf.len();
            let folder_name_bytes = get_folder_name().as_bytes().to_vec();
            buf.extend_from_slice(&get_checksum_bytes(folder_name_bytes).to_be_bytes().to_vec());
            Ok(buf.len() - initial_buf_len)
        }
    }

    fn get_length_as_u16_bytes(path: Vec<u8>) -> Vec<u8> {
        u16::try_from(path.clone().len())
            .unwrap()
            .to_be_bytes()
            .to_vec()
    }

    fn get_length_as_u32_bytes(path: Vec<u8>) -> Vec<u8> {
        u32::try_from(path.clone().len())
            .unwrap()
            .to_be_bytes()
            .to_vec()
    }

    fn get_folder_name() -> String {
        "/This/is/the/test/folderऐक".to_string()
    }

    fn get_file_name() -> String {
        "/This/is/the/test/folderऐक/File1".to_string()
    }

    fn get_file_data() -> String {
        "होली का त्यौहार बुराई पर अच्छाई की विजय का प्रतीक माना जाता है। इस त्यौहार से सीख लेते हुए हमें भी अपनी बुराइयों को छोड़ते हुए अच्छाई को अपनाना चाहिए। इस त्यौहार से एक और सीख मिलती है कि कभी भी हमें अहंकार नहीं करना चाहिए क्योंकि अहंकार हमारे सोचने समझने की शक्ति को बंद कर देता है।".to_string()
    }

    fn get_checksum_bytes(buf: Vec<u8>) -> u32 {
        let mut hasher = Hasher::new();
        hasher.update(&buf);
        hasher.finalize()
    }

    #[tokio::test]
    async fn test_create_folder_ok_read_data_from_stream() {
        initialize();
        let mut test_stream = TcpStreamTestStreamAllData {};

        let mut buf = vec![0b00000001u8];

        let folder_name_bytes = get_folder_name().as_bytes().to_vec();
        buf.extend_from_slice(&get_length_as_u16_bytes(folder_name_bytes.clone()));
        buf.extend_from_slice(&folder_name_bytes.clone());
        buf.extend_from_slice(&get_checksum_bytes(folder_name_bytes).to_be_bytes().to_vec());

        let number_of_bytes_read = buf.len();
        debug!("Buf created for test {:#?}", String::from_utf8(buf.clone()));

        match Server::create_parent_folder(&mut test_stream, &mut buf, number_of_bytes_read, 1)
            .await
        {
            Ok((buffer_state, folder_name_created)) => {
                // Assert bytes processed returned is equal to number of bytes in buffer happy case
                assert_eq!(buffer_state.start_index, number_of_bytes_read);
                // Assert bytes read during operation is equal to number of bytes in buffer
                // This will also assert we never called TcpStreamTestGood::read_data method, as we passed full buffer.
                assert_eq!(buffer_state.end_index, number_of_bytes_read);

                assert_eq!(
                    get_folder_name(),
                    folder_name_created[ROOT_SAVING_DIRECTORY.len()..]
                );
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
        match Server::create_parent_folder(&mut test_stream, &mut buf, number_of_bytes_read, 1)
            .await
        {
            Ok((buffer_state, folder_name_created)) => {
                // Assert bytes processed returned is equal to number of bytes in buffer happy case
                assert_eq!(buffer_state.start_index, buf.len());
                // Assert bytes read during operation is equal to number of bytes in buffer
                // This will also assert we never called TcpStreamTestGood::read_data method, as we passed full buffer.
                assert_eq!(buffer_state.end_index, buf.len());
                assert_eq!(
                    get_folder_name(),
                    folder_name_created[ROOT_SAVING_DIRECTORY.len()..]
                );
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
        let folder_name_bytes = get_folder_name().as_bytes().to_vec();
        buf.extend_from_slice(&get_length_as_u16_bytes(folder_name_bytes.clone()));

        let number_of_bytes_read = buf.len();
        match Server::create_parent_folder(&mut test_stream, &mut buf, number_of_bytes_read, 1)
            .await
        {
            Ok((buffer_state, folder_name_created)) => {
                // Assert bytes processed returned is equal to number of bytes in buffer happy case
                assert_eq!(buffer_state.start_index, buf.len());
                // Assert bytes read during operation is equal to number of bytes in buffer
                // This will also assert we never called TcpStreamTestGood::read_data method, as we passed full buffer.
                assert_eq!(buffer_state.end_index, buf.len());
                assert_eq!(
                    get_folder_name(),
                    folder_name_created[ROOT_SAVING_DIRECTORY.len()..]
                );
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
        let folder_name_bytes = get_folder_name().as_bytes().to_vec();
        buf.extend_from_slice(&get_length_as_u16_bytes(folder_name_bytes.clone()));
        buf.extend_from_slice(&folder_name_bytes.clone());

        let number_of_bytes_read = buf.len();
        match Server::create_parent_folder(&mut test_stream, &mut buf, number_of_bytes_read, 1)
            .await
        {
            Ok((buffer_state, folder_name_created)) => {
                // Assert bytes processed returned is equal to number of bytes in buffer happy case
                assert_eq!(buffer_state.start_index, buf.len());
                // Assert bytes read during operation is equal to number of bytes in buffer
                // This will also assert we never called TcpStreamTestGood::read_data method, as we passed full buffer.
                assert_eq!(buffer_state.end_index, buf.len());
                assert_eq!(
                    get_folder_name(),
                    folder_name_created[ROOT_SAVING_DIRECTORY.len()..]
                );
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

        let mut test_stream = TcpStreamTestStreamNoData {};

        let mut buf = vec![0b00000001u8];
        let number_of_bytes_read = buf.len();
        match Server::create_parent_folder(&mut test_stream, &mut buf, number_of_bytes_read, 1)
            .await
        {
            Ok(_) => {}
            Err(e) => {
                // EXPECTED
                print!("{}", e);
            }
        }
    }

    ///
    /// FILE TESTS
    ///

    #[tokio::test]
    async fn test_save_file_whole_frame_happy() {
        initialize();
        let mut test_stream = TcpStreamTestStreamNoData {};
        let mut buf = vec![0b00000010u8];

        // Add |file_name_length_bytes|file_name|checksum|
        let file_name_bytes = get_file_name().as_bytes().to_vec();
        buf.extend_from_slice(&get_length_as_u16_bytes(file_name_bytes.clone()));
        buf.extend_from_slice(&file_name_bytes.clone());
        buf.extend_from_slice(
            &get_checksum_bytes(file_name_bytes.clone())
                .to_be_bytes()
                .to_vec(),
        );

        // Add |file_data_bytes_length|file_data|checksum|
        let file_data = get_file_data().as_bytes().to_vec();
        buf.extend_from_slice(&get_length_as_u32_bytes(file_data.clone()));
        buf.extend_from_slice(&file_data.clone());
        buf.extend_from_slice(&get_checksum_bytes(file_data).to_be_bytes().to_vec());

        let number_of_bytes_read = buf.len();
        match Server::save_file(&mut test_stream, &mut buf, number_of_bytes_read, 1).await {
            Ok((buffer_state, file_path)) => {
                assert_eq!(buffer_state.start_index, buf.len());
                assert_eq!(buffer_state.end_index, buf.len());
                assert_eq!(get_file_name(), file_path[ROOT_SAVING_DIRECTORY.len()..]);
            }
            Err(e) => {
                print!("Error while saving file {}", e);
                panic!()
            }
        }
    }

    // Write |file_data_checksum|
    struct TcpStreamTestStreamFileDataChecksum {}

    impl ReadFromStream for TcpStreamTestStreamFileDataChecksum {
        async fn read_data(
            &mut self,
            buf: &mut Vec<u8>,
            _valid_read_index: usize,
        ) -> Result<usize, std::io::Error> {
            let initial_buf_len = buf.len();
            let file_bytes = get_file_data().as_bytes().to_vec();
            debug!(
                "Setting Checksum bytes {}",
                get_checksum_bytes(file_bytes.clone())
            );
            buf.extend_from_slice(&get_checksum_bytes(file_bytes).to_be_bytes().to_vec());
            debug!("{:#?}", buf.len());
            Ok(buf.len() - initial_buf_len) // Return no of new bytes added
        }
    }

    /// Tests the case where buffer of 1024*i32 doesn't hold the file data completely, and save_file
    /// has to read data from stream.
    #[tokio::test]
    async fn test_save_file_read_from_stream_checksum() {
        initialize();
        let mut test_stream = TcpStreamTestStreamFileDataChecksum {};
        let mut buf = vec![0b00000010u8];

        // Add |file_name_length_bytes|file_name|checksum|
        let file_name_bytes = get_file_name().as_bytes().to_vec();
        buf.extend_from_slice(&get_length_as_u16_bytes(file_name_bytes.clone()));
        buf.extend_from_slice(&file_name_bytes.clone());
        buf.extend_from_slice(
            &get_checksum_bytes(file_name_bytes.clone())
                .to_be_bytes()
                .to_vec(),
        );

        // Add |file_data_bytes_length|file_data|checksum|
        let file_data = get_file_data().as_bytes().to_vec();
        buf.extend_from_slice(&get_length_as_u32_bytes(file_data.clone()));
        buf.extend_from_slice(&file_data.clone());

        let number_of_bytes_read = buf.len();

        match Server::save_file(&mut test_stream, &mut buf, number_of_bytes_read, 1).await {
            Ok((buffer_state, file_path)) => {
                assert_eq!(buffer_state.start_index, buf.len());
                assert_eq!(buffer_state.end_index, buf.len());
                assert_eq!(get_file_name(), file_path[ROOT_SAVING_DIRECTORY.len()..]);
            }
            Err(e) => {
                print!("Error while saving file {}", e);
                panic!()
            }
        }
    }

    // Write |file_data|file_data_checksum|
    struct TcpStreamTestStreamFileDataAndChecksum {}

    impl ReadFromStream for TcpStreamTestStreamFileDataAndChecksum {
        async fn read_data(
            &mut self,
            buf: &mut Vec<u8>,
            valid_read_index: usize,
        ) -> Result<usize, std::io::Error> {
            let mut bytes_written = 0;
            let file_bytes = get_file_data().as_bytes().to_vec();

            if valid_read_index == 0 {
                buf.resize(file_bytes.len(), 0b00000000u8);
                buf.copy_from_slice(&file_bytes.clone());
                bytes_written = buf.len();
            } else {
                buf.extend_from_slice(&file_bytes.clone());
                bytes_written = file_bytes.len();
            }

            debug!(
                "Setting Checksum bytes {}",
                get_checksum_bytes(file_bytes.clone())
            );
            buf.extend_from_slice(&get_checksum_bytes(file_bytes).to_be_bytes().to_vec());
            bytes_written += 4;
            debug!("{:#?}", buf.len());
            Ok(bytes_written) // Return no of new bytes added
        }
    }

    /// Tests the case where buffer of 1024*i32 doesn't hold the file data completely, and save_file
    /// has to read data from stream.
    #[tokio::test]
    async fn test_save_file_read_from_stream_file_data_and_checksum() {
        initialize();
        let mut test_stream = TcpStreamTestStreamFileDataAndChecksum {};
        let mut buf = vec![0b00000010u8];

        // Add |file_name_length_bytes|file_name|checksum|
        let file_name_bytes = get_file_name().as_bytes().to_vec();

        buf.extend_from_slice(&get_length_as_u16_bytes(file_name_bytes.clone()));
        buf.extend_from_slice(&file_name_bytes.clone());
        buf.extend_from_slice(
            &get_checksum_bytes(file_name_bytes.clone())
                .to_be_bytes()
                .to_vec(),
        );

        // Add |file_data_bytes_length|file_data|checksum|
        let file_data = get_file_data().as_bytes().to_vec();
        buf.extend_from_slice(&get_length_as_u32_bytes(file_data.clone()));

        let number_of_bytes_read = buf.len();

        match Server::save_file(&mut test_stream, &mut buf, number_of_bytes_read, 1).await {
            Ok((buffer_state, file_path)) => {
                assert_eq!(buffer_state.start_index, buf.len());
                assert_eq!(buffer_state.end_index, buf.len());
                assert_eq!(get_file_name(), file_path[ROOT_SAVING_DIRECTORY.len()..]);
            }
            Err(e) => {
                print!("Error while saving file {}", e);
                panic!()
            }
        }
    }

    // Write |file_data_length|file_data|file_data_checksum|
    struct TcpStreamTestStreamFileDataLengthAndFileDataAndChecksum {}

    impl ReadFromStream for TcpStreamTestStreamFileDataLengthAndFileDataAndChecksum {
        async fn read_data(
            &mut self,
            buf: &mut Vec<u8>,
            _valid_read_index: usize,
        ) -> Result<usize, std::io::Error> {
            let initial_buf_len = buf.len();
            let file_bytes = get_file_data().as_bytes().to_vec();
            buf.extend_from_slice(&get_length_as_u32_bytes(file_bytes.clone()));
            buf.extend_from_slice(&file_bytes.clone());
            buf.extend_from_slice(&get_checksum_bytes(file_bytes).to_be_bytes().to_vec());

            debug!("{:#?}", buf.len());
            Ok(buf.len() - initial_buf_len) // Return no of new bytes added
        }
    }
    /// Tests the case where buffer of 1024*i32 doesn't hold the file data completely, and save_file
    /// has to read data from stream.
    #[tokio::test]
    async fn test_save_file_read_from_stream_file_length_and_file_data_and_checksum() {
        initialize();
        let mut test_stream = TcpStreamTestStreamFileDataLengthAndFileDataAndChecksum {};
        let mut buf = vec![0b00000010u8];

        // Add |file_name_length_bytes|file_name|checksum|
        let file_name_bytes = get_file_name().as_bytes().to_vec();
        buf.extend_from_slice(&get_length_as_u16_bytes(file_name_bytes.clone()));
        buf.extend_from_slice(&file_name_bytes.clone());
        buf.extend_from_slice(
            &get_checksum_bytes(file_name_bytes.clone())
                .to_be_bytes()
                .to_vec(),
        );

        let number_of_bytes_read = buf.len();

        match Server::save_file(&mut test_stream, &mut buf, number_of_bytes_read, 1).await {
            Ok((buffer_state, file_path)) => {
                assert_eq!(buffer_state.start_index, buf.len());
                assert_eq!(buffer_state.end_index, buf.len());
                assert_eq!(get_file_name(), file_path[ROOT_SAVING_DIRECTORY.len()..]);
            }
            Err(e) => {
                print!("Error while saving file {}", e);
                panic!()
            }
        }
    }

    #[test]
    /// Writes create parent folder request frame into temp file
    /// | 0x00000001|folder_name_length_bytes|folder_name|checksum|
    fn write_create_parent_folder_request_bytes() {
        initialize();
        let mut buf = vec![0b00000001u8];
        let folder_name_bytes = get_folder_name().as_bytes().to_vec();
        buf.extend_from_slice(&get_length_as_u16_bytes(folder_name_bytes.clone()));
        buf.extend_from_slice(&folder_name_bytes.clone());
        buf.extend_from_slice(&get_checksum_bytes(folder_name_bytes).to_be_bytes().to_vec());

        let mut file = File::create("/tmp/create_folder_request_raw").unwrap();
        file.write_all(&buf);
    }

    #[test]
    /// Writes create file request frame into temp file
    /// | 0x00000002|file_name_length_bytes|file_name|checksum|file_data_length_bytes|file_data|checksum_file_data|
    fn write_create_file_request_bytes() {
        initialize();
        let mut buf = vec![0b00000010u8];
        let file_name_bytes = get_file_name().as_bytes().to_vec();
        buf.extend_from_slice(&get_length_as_u16_bytes(file_name_bytes.clone()));
        buf.extend_from_slice(&file_name_bytes.clone());
        buf.extend_from_slice(
            &get_checksum_bytes(file_name_bytes.clone())
                .to_be_bytes()
                .to_vec(),
        );

        // Add |file_data_bytes_length|file_data|checksum|
        let file_data = get_file_data().as_bytes().to_vec();
        buf.extend_from_slice(&get_length_as_u32_bytes(file_data.clone()));
        buf.extend_from_slice(&file_data.clone());
        buf.extend_from_slice(&get_checksum_bytes(file_data).to_be_bytes().to_vec());

        let mut file = File::create("/tmp/create_file_request_raw").unwrap();
        file.write_all(&buf);
    }
}
