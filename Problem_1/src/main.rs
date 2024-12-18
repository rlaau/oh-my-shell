use nix::unistd::{fork, ForkResult, dup2, close, pipe, execvp};
use nix::sys::wait::{waitpid, WaitStatus};
use std::ffi::CString;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::io::RawFd;
use std::fs::File;
use std::env; // cd 명령을 위해 추가
use std::os::unix::io::IntoRawFd; // 추가 필요
#[derive(Debug)]
enum InputType {
    SingleCommand(Command),
    Pipe(Vec<Command>),
    InputRedirect(Command, String),
    OutputRedirect(Command, String),
}

#[derive(Debug, Clone)]
struct Command {
    pub program: String,
    pub args: Vec<String>,
}

fn parse_input(input: &str) -> Option<InputType> {
    let input = input.trim();
    if let Some((cmd, file)) = input.split_once('<') {
        // 입력 리디렉션
        return Some(InputType::InputRedirect(
            parse_command(cmd),
            file.trim().to_string(),
        ));
    }
    if let Some((cmd, file)) = input.split_once('>') {
        // 출력 리디렉션
        return Some(InputType::OutputRedirect(
            parse_command(cmd),
            file.trim().to_string(),
        ));
    }
    // 파이프 처리
    let parts = input.split('|');
    let mut commands = Vec::new();
    for part in parts {
        let trimmed_part = part.trim();
        if !trimmed_part.is_empty() {
            commands.push(parse_command(trimmed_part));
        }
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

fn run_single_command(cmd: &Command, input_file: Option<&str>, output_file: Option<&str>) {
    let c_program = CString::new(cmd.program.as_str()).expect("CString failed");
    let mut c_args: Vec<CString> = Vec::new();
    c_args.push(c_program.clone());
    c_args.extend(cmd.args.iter().map(|arg| CString::new(arg.as_str()).unwrap()));

    // 파일 리디렉션 처리
    if let Some(file) = input_file {
        let input_fd = File::open(file).expect("Failed to open input file");
        dup2(input_fd.as_raw_fd(), 0).expect("Failed to redirect input");
    }
    if let Some(file) = output_file {
        let output_fd = File::create(file).expect("Failed to create output file");
        dup2(output_fd.as_raw_fd(), 1).expect("Failed to redirect output");
    }

    execvp(&c_program, &c_args).expect("Failed to execute command");
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
                Ok(_) => println!("[oh-my-shell] Changed directory to {}", new_dir),
                Err(e) => eprintln!("cd: {}", e),
            }
            continue;
        }

        let parsed_input = parse_input(input);
        if let Some(input_type) = parsed_input {
            match input_type {
                InputType::SingleCommand(cmd) => {
                    handle_single_command(cmd, None, None);
                }
                InputType::InputRedirect(cmd, file) => {
                    handle_single_command(cmd, Some(&file), None);
                }
                InputType::OutputRedirect(cmd, file) => {
                    handle_single_command(cmd, None, Some(&file));
                }
                InputType::Pipe(commands) => {
                    handle_pipes(commands);
                }
            }
        }
    }
}

fn handle_single_command(cmd: Command, input_file: Option<&str>, output_file: Option<&str>) {
    match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            run_single_command(&cmd, input_file, output_file);
        }
        Ok(ForkResult::Parent { child }) => {
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
        Err(e) => eprintln!("Fork failed: {}", e),
    }
}

fn handle_pipes(commands: Vec<Command>) {
    // 파이프 라인 처리
    let mut prev_pipe: Option<RawFd> = None;
    let mut children = Vec::new();

    for (i, cmd) in commands.iter().enumerate() {
        let (r, w) = if i < commands.len() - 1 {
            let (r, w) = pipe().expect("Failed to create pipe");
            let rfd = r.into_raw_fd();
            let wfd = w.into_raw_fd();
            (Some(rfd), Some(wfd))
        } else {
            (None, None)
        };

        match unsafe { fork() } {
            Ok(ForkResult::Child) => {
                // 자식 프로세스
                // 이전 파이프의 읽기 끝이 있다면 표준 입력으로 연결
                if let Some(fd) = prev_pipe {
                    dup2(fd, 0).expect("Failed to dup2 input");
                    close(fd).expect("Failed to close old input fd");
                }
                // 다음 명령을 위해 현재 명령의 표준 출력을 파이프의 쓰기 끝으로 연결
                if let Some(fd) = w {
                    dup2(fd, 1).expect("Failed to dup2 output");
                    close(fd).expect("Failed to close write end of pipe");
                }

                // 읽기용 새 파이프 fd는 자식 프로세스에서 필요 없으므로 닫는다.
                if let Some(fd) = r {
                    close(fd).expect("Failed to close read end of pipe in child");
                }

                run_single_command(cmd, None, None);
                std::process::exit(0);
            }
            Ok(ForkResult::Parent { child }) => {
                children.push(child);

                // 부모 프로세스는 이전 명령을 위해 쓴 파이프의 쓰기 끝은 닫아야 한다.
                if let Some(fd) = w {
                    close(fd).expect("Failed to close write fd in parent");
                }

                // 다음 명령에서 사용할 읽기 끝을 prev_pipe에 저장
                prev_pipe = r;
            }
            Err(e) => eprintln!("Fork failed: {}", e),
        }
    }

    for child in children {
        waitpid(child, None).expect("Failed to wait for child");
    }
}
