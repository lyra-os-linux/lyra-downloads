pub mod client;
pub mod errors;
pub mod process;

pub use client::{Aria2Client, Aria2Error, Aria2Options, Aria2Result, Aria2Status};
pub use process::{Aria2Process, SpawnConfig, SpawnError};

/// Opções por tarefa para o perfil de conexões escolhido. O perfil 16
/// solicita 16 em `max-connection-per-server` e `split` (máximo aceito pelo
/// aria2 1.37 para `max-connection-per-server`). O número é um teto: o
/// servidor pode aceitar menos conexões.
pub fn connection_options(connections: u8) -> Aria2Options {
    let n = connections.clamp(1, 16).to_string();
    let mut o = Aria2Options::new();
    o.insert("max-connection-per-server".into(), n.clone());
    o.insert("split".into(), n);
    o.insert(
        "min-split-size".into(),
        process::DEFAULT_MIN_SPLIT_SIZE.into(),
    );
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_16_requests_16_in_both() {
        let o = connection_options(16);
        assert_eq!(o["max-connection-per-server"], "16");
        assert_eq!(o["split"], "16");
        assert_eq!(o["min-split-size"], "1M");
    }

    #[test]
    fn profiles_map_one_to_one() {
        for n in [1u8, 4, 8, 16] {
            let o = connection_options(n);
            assert_eq!(o["split"], n.to_string());
            assert_eq!(o["max-connection-per-server"], n.to_string());
        }
    }
}
