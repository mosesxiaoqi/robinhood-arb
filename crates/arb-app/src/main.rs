use arb_adapters::rpc::{RpcOptions, RpcSource};
use arb_app::{config::Config, ingest::collect};
use std::{
    env, fs,
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

fn load(path: &str) -> Result<Config, String> {
    let text = fs::read_to_string(path).map_err(|_| "cannot read configuration file".to_owned())?;
    Config::parse(&text).map_err(|e| e.to_string())
}
async fn run(args: &[String]) -> Result<(), String> {
    match args {
        [command,flag,path] if command=="check-config" && flag=="--config" => {
            load(path)?;println!("configuration valid (offline validation; network not contacted)");Ok(())
        }
        [command,flag,path,from_flag,from,to_flag,to] if command=="collect" && flag=="--config" && from_flag=="--from" && to_flag=="--to" => {
            let config=load(path)?;
            let from=from.parse().map_err(|_|"invalid --from block")?;let to=to.parse().map_err(|_|"invalid --to block")?;
            if let Some(parent)=config.database.parent().filter(|p|!p.as_os_str().is_empty()) {fs::create_dir_all(parent).map_err(|_|"cannot create data directory")?;}
            let stamp=SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_|"invalid system clock")?.as_nanos();
            let options=RpcOptions {chain_id:config.chain_id,source:"rpc".into(),run_id:format!("{}-{stamp}",std::process::id()),requests_per_second:config.requests_per_second,max_concurrency:config.max_concurrency,retry_limit:config.retry_limit,timeout_ms:config.timeout_ms,max_response_bytes:config.max_response_bytes};
            let source=RpcSource::new(&config.rpc_url,options).map_err(|e|e.to_string())?;
            let count=collect(&source,&config.database,from,to,config.queue_capacity).await.map_err(|e|e.to_string())?;
            println!("committed {count} complete blocks");Ok(())
        }
        [command,mode_flag,mode,config_flag,path,checkpoint_flag,id,to_flag,to] if command=="replay" && mode_flag=="--mode" && matches!(mode.as_str(),"chain"|"observed") && config_flag=="--config" && checkpoint_flag=="--checkpoint" && to_flag=="--to" => {
            let config=load(path)?;let id=id.parse().map_err(|_|"invalid checkpoint id")?;let to=to.parse().map_err(|_|"invalid replay endpoint")?;
            let observed=mode=="observed";
            let count=tokio::task::spawn_blocking(move || arb_app::replay::replay_checkpoint_mode(config,id,to,observed)).await.map_err(|_|"replay worker failed")?.map_err(|e|e.to_string())?;
            println!("replayed {count} candidates (stored historical input only)");Ok(())
        }
        _=>Err("usage: arb-app check-config --config <path> | collect --config <path> --from <block> --to <block> | replay --mode <chain|observed> --config <path> --checkpoint <id> --to <block>".into()),
    }
}
#[tokio::main]
async fn main() -> ExitCode {
    match run(&env::args().skip(1).collect::<Vec<_>>()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
