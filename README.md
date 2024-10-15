## What it is
Tool to copy folders from between two computers under same network eg. two laptops connected to same WIFI.

## How to run

Host_A(client) ---data--> Host_B(server)

### Start server first on Host_B
```
$cargo run server
```

### Run client on Host_A
```
$cargo run client <folder_complete_path> <ip_address>
eg. 
cargo run client "/Users/vikaspathania/Documents/books" 10.0.0.129
```
You can run client as much time giving the different folder names you want to copy

## Next
- Add protocol for peer-to-peer transfer over internet. (new library)
- Add download API
- add peer to peer storage protocol
    - have a network of peers available on every host
    - save data in any random set of hosts
    - keep list of hosts to help in retreiving data
    - To avoid data loss, copy over two locations.
    - provide encryption support (Authentication keys)
- add peer to peer storage selective nodes
