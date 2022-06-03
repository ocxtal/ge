
use clap::Parser;
use std::process::Command;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    pattern: String,

    #[clap(short, long)]
    context: Option<usize>,

    #[clap(short, long)]
    before: Option<usize>,

    #[clap(short, long)]
    after: Option<usize>,

    #[clap(short, long)]
    separator: Option<String>,
}

fn main() {
    let args = Args::parse();

    let mut grep_args = vec![
        "grep".to_string(),
        "--color=never".to_string(),
        "--line-number".to_string()
    ];
    if let Some(c) = args.context {
        grep_args.push(format!("--context={}", c));
    }
    if let Some(b) = args.context {
        grep_args.push(format!("--before={}", b));
    }
    if let Some(a) = args.context {
        grep_args.push(format!("--after={}", a));
    }
    grep_args.push(args.pattern.clone());

    println!("{:?}", grep_args);

    let grep = Command::new("git").args(&grep_args).output();
    if grep.is_err() {
        panic!("failed to get output of \"git grep\"");
    }

    println!("{:?}, {:?}", args, grep.unwrap());
}
