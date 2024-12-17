use std::{io, thread::sleep, time::Duration};
use std::io::{Write}; // Write trait 가져오기
fn main() {
    println!("######### oh-my-shell starts! #########");
    loop {
        print!(">>>");
        io::stdout().flush().expect("Failed to flush stdout");

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .expect("Failed to read line");

        let input = input.trim(); // 개행 문자 제거
        if input == "exit" {
            println!("Exit oh-my-shell. Bye!");
            break;
        }

        println!("You entered: {}", input);
        
    }
}
