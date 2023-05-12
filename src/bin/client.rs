use std::{unreachable, io, path::PathBuf};

use anyhow::bail;
use simple_file_transfer_v2::{fs::browser::{Request, Response}, read_input};
use tokio::{net::TcpStream, io::{BufStream, AsyncRead, AsyncWrite, AsyncWriteExt, AsyncReadExt}};

async fn make_request(stream: &mut (impl AsyncRead + AsyncWrite + Unpin), buffer: &mut Vec<u8>, request: Request) -> Result<Response, anyhow::Error> {
    let data = rmp_serde::to_vec(&request)?;

    stream.write_u16(data.len().try_into()?).await?;
    stream.write(&data).await?;
    stream.flush().await?;

    let request_len: usize = stream.read_u16().await?.into();
    if request_len > buffer.len() {
        buffer.resize(request_len, 0);
    }

    let slice = &mut buffer[..request_len];
    stream.read_exact(slice).await?;

    Ok(rmp_serde::from_slice(&slice)?)
}

fn ask_for_command_selection<S: AsRef<str>>(commands: &Vec<S>) -> Result<u32, io::Error> {
    let options = commands.iter()
        .enumerate()
        .map(|(idx, command)| format!("{} - {}\n", idx + 1, command.as_ref()))
        .collect::<Vec<String>>()
        .concat();

    let number_of_commands = commands.len().try_into().unwrap();

    loop {
        println!("Commands:\n{options}");

        match read_input(Some("Select a command: "))?
            .trim()
            .parse::<u32>()
        {
            Ok(number) if number >= 1 && number <= number_of_commands => break Ok(number),
            _ => println!("Invalid selection, please try again!\n")
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    const SIZE: usize = 4096;
    let mut buffer = vec![0; SIZE];
    let mut stream = BufStream::new(TcpStream::connect("127.0.0.1:8000").await?);

    println!("Connected!");

    let root_commands = vec!["Create Cursor", "Delete Cursor", "List Cursors", "Select Cursor", "Exit"];
    let cursor_commands = vec!["Read", "Move", "Get Location", "Deselect"];

    let mut cursors = vec![];
    let mut selected_cursor = None;

    loop {
        if let Some(id) = selected_cursor {
            let commands = cursor_commands.clone();
            match tokio::task::spawn_blocking(move || ask_for_command_selection(&commands)).await?? {
                1 => {
                    match make_request(&mut stream, &mut buffer, Request::Read { id }).await? {
                        Response::Read(Ok(elements)) => {
                            println!("Elements:\n{}", elements.into_iter()
                                .map(|element| format!("{} - Name: {}\t\tSize: {}\tCreated: {}\tModified: {}\n",
                                    if element.is_file {"F"} else {"D"},
                                    element.name.to_string_lossy(),
                                    element.size,
                                    element.created.unwrap(),
                                    element.modified.unwrap()
                                ))
                                .collect::<Vec<String>>()
                                .concat()
                            )
                        }
                        Response::Read(Err(err)) => {
                            println!("Error while attempting to read cursor: {err}\n");
                        }
                        _ => bail!("Unexpected response type")
                    }
                }
                2 => {
                    let path: PathBuf = tokio::task::spawn_blocking(move || read_input(Some("New Path: ")))
                        .await??
                        .into();

                    match make_request(&mut stream, &mut buffer, Request::Move { id, path: path.clone() }).await? {
                        Response::Move(Ok(())) => {
                            println!("Moved to {path:?}\n");
                        }
                        Response::Move(Err(err)) => {
                            println!("Error while attempting to move cursor: {err}\n");
                        }
                        _ => bail!("Unexpected response type")
                    }
                }
                3 => {
                    match make_request(&mut stream, &mut buffer, Request::GetLocation { id }).await? {
                        Response::GetLocation(Ok(path)) => {
                            println!("Cursor is at {path:?}\n");
                        }
                        Response::GetLocation(Err(err)) => {
                            println!("Error while attempting to get cursor location: {err}\n");
                        }
                        _ => bail!("Unexpected response type")
                    }
                }
                4 => {
                    selected_cursor = None;
                }
                _ => unreachable!()
            }
        } else {
            let commands = root_commands.clone();
            match tokio::task::spawn_blocking(move || ask_for_command_selection(&commands)).await?? {
                1 => {
                    match make_request(&mut stream, &mut buffer, Request::Create).await? {
                        Response::Create(Ok(id)) => {
                            println!("Cursor {id} created!\n");
                            cursors.push(id);
                        }
                        Response::Create(Err(err)) => {
                            println!("Error while attempting to create cursor: {err}\n");
                        }
                        _ => bail!("Unexpected response type")
                    }
                }
                2 => {
                    if cursors.is_empty() {
                        println!("There are no cursors to delete!\n");
                        continue;
                    }

                    println!("Select a cursor to delete:");
                    let commands: Vec<String> = cursors.iter()
                        .map(|id| format!("Cursor {id}"))
                        .collect();

                    let selection: usize = tokio::task::spawn_blocking(move || ask_for_command_selection(&commands)).await??
                        .try_into()
                        .unwrap();

                    match make_request(&mut stream, &mut buffer, Request::Destroy { id: cursors[selection - 1] }).await? {
                        Response::Destroy(Ok(())) => {
                            println!("Cursor {selection} successfully destroyed\n");
                            cursors.remove(selection - 1);
                        }
                        Response::Destroy(Err(err)) => {
                            println!("Error while attempting to delete cursor: {err}\n");
                        }
                        _ => bail!("Unexpected response type")
                    }
                }
                3 => {
                    if cursors.is_empty() {
                        println!("There are no cursors\n");
                    } else {
                        println!("Cursors:\n{}", cursors.iter()
                            .map(|id| format!("Cursor {id}\n"))
                            .collect::<Vec<String>>()
                            .concat()
                        )
                    }
                }
                4 => {
                    if cursors.is_empty() {
                        println!("There are no cursors to select!\n");
                        continue;
                    }

                    println!("Select a cursor:");
                    let commands: Vec<String> = cursors.iter()
                        .map(|id| format!("Cursor {id}"))
                        .collect();

                    let selection: usize = tokio::task::spawn_blocking(move || ask_for_command_selection(&commands)).await??
                        .try_into()
                        .unwrap();
                    
                    let id = cursors[selection - 1];
                    println!("Selected cursor {id}\n");
                    selected_cursor = Some(id);
                }
                5 => break,
                _ => unreachable!()
            }
        }
    }

    Ok(())
}
