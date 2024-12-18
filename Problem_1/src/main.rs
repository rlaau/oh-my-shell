use nix::unistd::{fork, ForkResult, dup2, close, pipe, execvp};
use nix::sys::wait::{waitpid, WaitStatus};
use std::ffi::CString;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::io::{RawFd, IntoRawFd};
use std::fs::File;
use std::env; 

#[derive(Debug)]
enum InputType {
    SingleCommand(Command),
    Pipe(Vec<Command>),
    InputRedirect(Command, String),
    OutputRedirect(Command, String),
    BiRedirect(Command, String, String),
}

#[derive(Debug, Clone)]
struct Command {
    pub program: String,
    pub args: Vec<String>,
    pub input_file: Option<String>,
    pub output_file: Option<String>,
}


/// 리다이렉션이 포함될 수 있는 단일 명령어 문자열을 파싱하여,
/// 프로그램, 인자, input_file, output_file를 추출하는 함수.
fn parse_redir_command(input: &str) -> Option<Command> {
    // 우선 전체를 공백 기준으로 토큰화
    let tokens: Vec<&str> = input.trim().split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    let mut program = String::new();
    let mut args = Vec::new();
    let mut input_file: Option<String> = None;
    let mut output_file: Option<String> = None;

    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            "<" => {
                // 다음 토큰이 파일명
                if i + 1 < tokens.len() {
                    input_file = Some(tokens[i+1].to_string());
                    i += 2;
                } else {
                    eprintln!("Syntax error: no input file after '<'");
                    return None;
                }
            }
            ">" => {
                if i + 1 < tokens.len() {
                    output_file = Some(tokens[i+1].to_string());
                    i += 2;
                } else {
                    eprintln!("Syntax error: no output file after '>'");
                    return None;
                }
            }
            //커멘드 파싱하는 문구
            // (커멘드) > (커멘드) 의 구조일 것이니까
            // 토큰에 대해서 arg에 추가하는 것.
            token => {
                if program.is_empty() {
                    program = token.to_string();
                } else {
                    args.push(token.to_string());
                }
                i += 1;
            }
        }
    }

    if program.is_empty() {
        return None;
    }

    Some(Command {
        program,
        args,
        input_file,
        output_file,
    })
}

fn parse_input(input: &str) -> Option<InputType> {
    //재귀적 파싱
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    // 파이프 단위로 나누기
    let pipeline: Vec<&str> = input.split('|').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if pipeline.is_empty() {
        return None;
    }

    // 파이프가 하나도 없다면 단일 명령
    if pipeline.len() == 1 {
        let cmd = parse_redir_command(pipeline[0])?;
        // 파이프 없음, 명령 하나에 리다이렉션이 있을 수 있음.
        match (cmd.input_file.clone(), cmd.output_file.clone()) {
            // > 입력인 경우
            (Some(inf), None) => Some(InputType::InputRedirect(cmd, inf)),
            // < 입력인 경우
            (None, Some(outf)) => Some(InputType::OutputRedirect(cmd, outf)),
            (Some(inf), Some(outf)) => {
                // 재귀적으로 인터프리터 구현해야 하나 했는데, 그정돈 아님
                // 애초에 과제에서 요구하는 내용도 아니긴 하지만, 파일 리디렉션은 재귀가 무한 깊이가 불가능
                // 차피 파일에서 끝나므로, 한 단계의 처리, 즉 화살표는 기껏해야 최대 한 개인 한계를 이용.
                //cat < inp.txt > out.txt 이런 입력에 대한 처리.
                Some(InputType::BiRedirect(cmd, inf, outf))
            }
            //그냥 실행인 경우
            (None, None) => Some(InputType::SingleCommand(cmd))
        }
    } else {
        // 파이프가 2개 이상 있을 때
        let mut commands = Vec::new();
        for seg in pipeline {
            // 각 세그먼트, 즉 (커멘드)에 대해선
            // 가장 기본적인 선 처리만 해 두기
            // 그래도 이 정보 기반으로 다시 실행 가능.
            let cmd = parse_redir_command(seg)?;
            commands.push(cmd);
        }
        // 여기서는 파이프 벡터를 반환. 각 명령은 run_single_command에서 input/output_file을 처리 가능
        //파이프는 (커멘드) | (커멘드) | (커멘드)의 형태로 처리
        Some(InputType::Pipe(commands))
    }
}

fn run_single_command(cmd: &Command, input_file: Option<&str>, output_file: Option<&str>) {
    let c_program = CString::new(cmd.program.as_str()).expect("CString failed");
    let mut c_args: Vec<CString> = Vec::new();
    c_args.push(c_program.clone());
    c_args.extend(cmd.args.iter().map(|arg| CString::new(arg.as_str()).unwrap()));
    

    //여기서 쌍방향도 처리 가능.
    let infile = input_file.or(cmd.input_file.as_deref());
    let outfile = output_file.or(cmd.output_file.as_deref());
    
    //두 단계 연속으로 처리하게 하면 됨.
    // 파일 구조의 한계 덕분.
    if let Some(file) = infile {
        let input_fd = File::open(file).expect("Failed to open input file");
        dup2(input_fd.as_raw_fd(), 0).expect("Failed to redirect input");
    }
    if let Some(file) = outfile {
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

        //cd도 걍 구현해봄. 근데 굳이 구현 필욘 x.
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
                // 결국 모든 것은 커멘드의 조합임. 커멘드 실행 이후 방향 컨트롤의 문제
                // 파이프를 쓰지 않고, 파일에 의존하는 경우엔 싱긒_커멘드 인수 컨트롤로 충분.
                InputType::SingleCommand(cmd) => {
                    handle_single_command(cmd, None, None);
                }
                InputType::InputRedirect(cmd, file) => {
                    handle_single_command(cmd, Some(&file), None);
                }
                InputType::OutputRedirect(cmd, file) => {
                    handle_single_command(cmd, None, Some(&file));
                }
                InputType::BiRedirect(cmd, inf, outf) => {
                    handle_single_command(cmd,Some(&inf), Some(&outf));
                }
                
                //결국, 파이프가 아닌 싱글 커멘드들은 전부 위에서 처리됨
                // 여기서부턴 파이프를 이용
                // 근데, 파이프는 위의 로직을 걍 반복해주면 끝
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
    let mut prev_pipe: Option<RawFd> = None;
    let mut children = Vec::new();

    for (i, cmd) in commands.iter().enumerate() {
        //커멘드 순회하면서 파이프 생성 여부 결정
        // 끝에선 당연히 없음.
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
                // 대충 조건문으로 모든 케이스 검사
                // 현재 배열은 (커멘드), (커멘드), (커멘드)인데,
                // prev_pipe를 바탕으로 읽어서, write로 적은 후, r로 write이후를 포인팅.
                //파이프 모델 떠올리면 편함.
                // 파이프를 연결짓는단 마인드니까. 당연히 r은 prev로 갈 꺼고.
                // prev-(w-r)로 호출

                if let Some(fd) = prev_pipe {
                    dup2(fd, 0).expect("Failed to dup2 input");
                    close(fd).expect("Failed to close old input fd");
                }
                if let Some(fd) = w {
                    dup2(fd, 1).expect("Failed to dup2 output");
                    close(fd).expect("Failed to close write end of pipe");
                }
                if let Some(fd) = r {
                    close(fd).expect("Failed to close read end of pipe in child");
                }
                
                //해당 포인터를 바탕으로 커멘드 하나 실행
                run_single_command(cmd, None, None);
                std::process::exit(0);
            }
            Ok(ForkResult::Parent { child }) => {

                //위의 pre_pipe에 대한 if에서 이미 동기성이 만족.
                // pre_pipe는 이전의 r인데, 이전의 r은 이전의 w에 대한 블로킹 상태
                // 지금의 w는 지금의 prev_pipe이후 실행, prev_pipe는 이전의 r, 이전의 r은 이전의 w의존
                //그러니 자동으로 w->r->w-> 순서가 유지
                //굳이 wait필요없음.
                children.push(child);

                if let Some(fd) = w {
                    close(fd).expect("Failed to close write fd in parent");
                }

                prev_pipe = r;
            }
            Err(e) => eprintln!("Fork failed: {}", e),
        }
    }
    if let Some(fd) = prev_pipe {
        close(fd).expect("Failed to close last pipe read end in parent");
    }


    // 모든 자식의 종료 상태를 여기서 수집
    let mut results = Vec::new();
    for child in &children {
        let status = waitpid(*child, None).expect("Failed to wait for child");
        results.push(status);
    }


    for status in results {
        match status {
            WaitStatus::Exited(pid, code) => {
                println!("[oh-my-shell] Child process terminated: pid {}, status {}", pid, code);
            }
            WaitStatus::Signaled(pid, signal, _) => {
                println!("[oh-my-shell] Child process terminated by signal: pid {}, signal {:?}", pid, signal);
            }
            _ => println!("[oh-my-shell] Child process ended unexpectedly."),
        }
    }
}
