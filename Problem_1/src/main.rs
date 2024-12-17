use nix::unistd::{fork, ForkResult, dup2, close, pipe};
use nix::sys::wait::{waitpid, WaitStatus};
use std::ffi::CString;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::io::RawFd;
use std::env; // cd 명령을 위해 추가

#[derive(Debug)]
enum InputType {
    SingleCommand(Command),
    Pipe(Vec<Command>),
}

#[derive(Debug, Clone)]
struct Command {
    pub program: String,
    pub args: Vec<String>,
}

fn parse_input(input: &str) -> Option<InputType> {
    let parts = input.trim().split('|');
    let mut commands = Vec::new();

    for part in parts {
        let trimmed_part = part.trim();
        if trimmed_part.is_empty() {
            continue;
        }
        commands.push(parse_command(trimmed_part));
    }

    match commands.len() {
        0 => None,
        1 => Some(InputType::SingleCommand(commands[0].clone())),
        _ => Some(InputType::Pipe(commands)),
    }
}

fn parse_command(input: &str) -> Command {
    let mut tokens = input.trim().split_whitespace();
    let program = tokens.next().unwrap_or_default().to_string();
    let args = tokens.map(String::from).collect();
    Command { program, args }
}

fn run_single_command(cmd: &Command) {
    let c_program = CString::new(cmd.program.as_str()).expect("CString failed");

    let mut c_args: Vec<CString> = Vec::new();
    c_args.push(c_program.clone()); // 첫 번째 인자는 프로그램 이름
    c_args.extend(
        cmd.args
            .iter()
            .map(|arg| CString::new(arg.as_str()).unwrap()),
    );
    //TODO: 이거 지우기
    println!("{:?},{:?}",&c_program, &c_args);
    nix::unistd::execvp(&c_program, &c_args).expect("Failed to execute command");
}

fn main() {
    println!("######### oh-my-shell starts! #########");

    loop {
        print!(">>> ");
        io::stdout().flush().expect("Failed to flush stdout");

        let mut input = String::new();
        io::stdin().read_line(&mut input).expect("Failed to read line");
        let input = input.trim();

        if input == "exit" {
            println!("Exit oh-my-shell. Bye!");
            break;
        }

        if input.starts_with("cd") {
            let parts: Vec<&str> = input.split_whitespace().collect();
            let new_dir = parts.get(1).unwrap_or(&"/");

            match env::set_current_dir(new_dir) {
                Ok(_) => {
                    println!("[oh-my-shell] cd executed: Changed directory to {}", new_dir);
                    println!("[oh-my-shell] Child process terminated: pid {}, status 0", std::process::id());
                }
                Err(e) => {
                    eprintln!("cd: Failed to change directory: {}", e);
                    println!("[oh-my-shell] Child process terminated: pid {}, status 1", std::process::id());
                }
            }
            continue;
        }

        let parsed_input = parse_input(input);
        if let Some(input_type) = parsed_input {
            match input_type {
                InputType::SingleCommand(cmd) => {
                    handle_single_command(cmd);
                }
                InputType::Pipe(commands) => {
                    handle_pipes(commands);
                }
            }
        }
    }
}

fn handle_single_command(cmd: Command) {
    match unsafe { fork() } {
        Ok(ForkResult::Child) => run_single_command(&cmd),
        Ok(ForkResult::Parent { child }) => {
            match waitpid(child, None).expect("Failed to wait for child") {
                WaitStatus::Exited(pid, status) => {
                    println!("\n[oh-my-shell] Child process terminated: pid {}, status {}", pid, status);
                }
                WaitStatus::Signaled(pid, signal, _) => {
                    println!("\n[oh-my-shell] Child process terminated by signal: pid {}, signal {:?}", pid, signal);
                }
                _ => println!("\n[oh-my-shell] Child process ended unexpectedly."),
            }
        }
        Err(e) => eprintln!("Fork failed: {}", e),
    }
}

fn handle_pipes(commands: Vec<Command>) {
    let mut prev_pipe: Option<RawFd> = None;
    let mut children = Vec::new();

    for (i, cmd) in commands.iter().enumerate() {
        let (read_fd, write_fd): (Option<RawFd>, Option<RawFd>) = if i < commands.len() - 1 {
            let (r, w) = pipe().expect("Failed to create pipe");
            (Some(r.as_raw_fd()), Some(w.as_raw_fd()))
        } else {
            (None, None)
        };

        match unsafe { fork() } {
            Ok(ForkResult::Child) => {
                // 이전 파이프를 입력으로 설정
                if let Some(fd) = prev_pipe {
                    dup2(fd, 0).expect("Failed to dup2 input");
                    close(fd).unwrap();
                }
                // 다음 파이프를 출력으로 설정
                if let Some(fd) = write_fd {
                    dup2(fd, 1).expect("Failed to dup2 output");
                    close(fd).unwrap();
                }
                run_single_command(cmd);
            }
            Ok(ForkResult::Parent { child }) => {
                children.push(child);
                // 부모 프로세스가 파이프의 끝을 닫아줌
                if let Some(fd) = prev_pipe {
                    close(fd).unwrap();
                }
                if let Some(fd) = write_fd {
                    close(fd).unwrap();
                }
                prev_pipe = read_fd;
            }
            Err(e) => {
                eprintln!("fork failed: {}", e);
                return;
            }
        }
    }

    // 부모 프로세스는 모든 자식이 종료될 때까지 대기
    for child in children {
        match waitpid(child, None).expect("Failed to wait for child") {
            WaitStatus::Exited(pid, status) => {
                println!("[oh-my-shell] Child process terminated: pid {}, status {}", pid, status);
            }
            WaitStatus::Signaled(pid, signal, _) => {
                println!("[oh-my-shell] Child process terminated by signal: pid {}, signal {:?}", pid, signal);
            }
            _ => println!("[oh-my-shell] Child process ended unexpectedly."),
        }
    }
}
