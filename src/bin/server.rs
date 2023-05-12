use std::{net::SocketAddr, io};

use simple_file_transfer_v2::{fs::{browser::{Browser, Request}, mapped_fs::MappedFS}, read_input};
use tokio::{io::{AsyncReadExt, AsyncWriteExt, BufStream}, net::{TcpListener, TcpStream}, signal, sync::watch::{self, Receiver}};

fn run_cli(mut mapped_fs: MappedFS) -> Result<(), anyhow::Error> {
    loop {
        let input = read_input(Some("Enter a command: "))?;
        match input.to_lowercase().as_str() {
            "exit" => return Ok(()),
            "add" => {
                let path = read_input(Some("Enter an absolute path: "))?;
                match mapped_fs.add(&path) {
                    Ok(_) => println!("Successfully added the path {path}"),
                    Err(err) => println!("Error: {err}"),
                }
            }
            _ => ()
        }
    }
}

async fn handle_socket(mut rx: Receiver<bool>, socket: TcpStream, _address: SocketAddr, fs: MappedFS) -> Result<(), anyhow::Error> {
    tokio::select! {
        result = async move {
            let mut browser = Browser::new(16, fs);

            const SIZE: usize = 4096;
            let mut buffer = vec![0; SIZE];
            let mut stream = BufStream::new(socket);
            loop {
                let request_len: usize = stream.read_u16().await?.into();
                if request_len > buffer.len() {
                    buffer.resize(request_len, 0);
                }

                let slice = &mut buffer[..request_len];
                stream.read_exact(slice).await?;

                let request: Request = rmp_serde::from_slice(&slice)?;
                let response = browser.process(request).await;
                let response = rmp_serde::to_vec(&response)?;

                stream.write_u16(response.len().try_into().unwrap()).await?;
                stream.write_all(&response).await?;
                stream.flush().await?;
            }
        } => result,
        _ = rx.changed() => Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let mapped_fs = MappedFS::new();

    let mapped_fs_for_cli = mapped_fs.clone();
    let cli_future = tokio::task::spawn_blocking(|| run_cli(mapped_fs_for_cli));

    let (tx, rx) = watch::channel(true);

    tokio::task::spawn(async move {
        let listener = match TcpListener::bind("127.0.0.1:8000").await {
            Ok(listener) => listener,
            Err(error) => return Err::<(), io::Error>(error),
        };

        let mut rx = rx;
        let rx2 = rx.clone();
        tokio::select! {
            _ = async move {
                loop {
                    match listener.accept().await {
                        Ok((socket, address)) => {
                            println!("Connection recieved from {address}");
                            
                            tokio::spawn(handle_socket(rx2.clone(), socket, address, mapped_fs.clone()));
                        }
                        Err(error) => {
                            println!("Error: {error}");
                        }
                    }
                }
            } => Ok(()),
            _ = rx.changed() => Ok(())
        }
    });

    tokio::select! {
        _ = signal::ctrl_c() => {},
        _ = cli_future => {}
    }

    _ = tx.send(false);
    tx.closed().await;

    Ok(())
}
