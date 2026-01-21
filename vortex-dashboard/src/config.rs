use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version)]
pub struct Args {
    #[arg(short, long, default_value_t = 16)]
    pub shards: usize,
    
    #[arg(short, long, default_value = "./data")]
    pub dir: String,

    #[arg(short, long, default_value_t = 1_000_000)]
    pub capacity: usize,

    #[arg(short, long, default_value_t = 9000)]
    pub port: u16,

    #[arg(short, long)]
    pub clean: bool,
}
