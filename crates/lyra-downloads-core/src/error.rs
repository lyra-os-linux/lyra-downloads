use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("erro de banco de dados: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("erro de I/O em {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("tarefa não encontrada: {0}")]
    TaskNotFound(uuid::Uuid),

    #[error("nome de arquivo inválido: {0}")]
    InvalidFilename(String),

    #[error("destino inválido: {0}")]
    InvalidDestination(String),

    #[error("URL inválida: apenas http:// e https:// são aceitas ({0})")]
    UnsupportedUrlScheme(String),

    #[error("migração de banco de dados incompatível: versão do arquivo ({found}) mais nova que a suportada por este binário ({supported})")]
    IncompatibleSchema { found: i64, supported: i64 },
}

pub type CoreResult<T> = Result<T, CoreError>;
