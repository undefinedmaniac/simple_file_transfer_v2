use std::io::{self, Write, BufRead};

pub mod fs;

pub fn read_input(prompt: Option<&str>) -> Result<String, io::Error> {
    if let Some(prompt) = prompt {
        print!("{}", prompt);
        io::stdout().flush()?;
    }

    // Obtain one line and leave off the \n
    Ok(io::stdin().lock().lines().nth(0).unwrap()?)
}
