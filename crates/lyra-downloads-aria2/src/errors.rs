//! Tradução dos códigos de erro do aria2 (tabela "Exit status" do manual,
//! conferida para a versão 1.37.0) para mensagens em português.
//!
//! As mensagens descrevem o que o motor observou, sem afirmar causas que o
//! aria2 não consegue determinar (ex.: um 403 pode ser link expirado,
//! bloqueio por região, falta de cookie de sessão, limite do servidor...).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    /// Vale a pena tentar de novo com o mesmo link.
    Transitorio,
    /// Tentar de novo com o mesmo link provavelmente falha; talvez um novo link resolva.
    Link,
    /// Problema local (disco, permissão, destino).
    Local,
    Outro,
}

pub struct ErrorDescription {
    pub message: &'static str,
    pub category: ErrorCategory,
}

pub fn describe(code: u32) -> ErrorDescription {
    use ErrorCategory::*;
    let (message, category) = match code {
        2 => ("Tempo de espera esgotado ao falar com o servidor.", Transitorio),
        3 => ("Recurso não encontrado no servidor (por exemplo, HTTP 404).", Link),
        4 => ("O servidor respondeu \"não encontrado\" repetidamente.", Link),
        5 => ("A velocidade ficou abaixo do mínimo configurado.", Transitorio),
        6 => ("Problema de rede ao baixar.", Transitorio),
        8 => ("O servidor não aceita retomar a partir do ponto parcial.", Link),
        9 => ("Espaço em disco insuficiente no destino.", Local),
        10 => ("O arquivo de controle .aria2 não corresponde ao download.", Local),
        11 => ("Este arquivo já está sendo baixado por outra tarefa.", Local),
        13 => ("Já existe um arquivo com este nome no destino.", Local),
        14 => ("Não foi possível renomear o arquivo.", Local),
        15 => ("Não foi possível abrir o arquivo parcial existente.", Local),
        16 => ("Não foi possível criar o arquivo no destino (verifique permissões).", Local),
        17 => ("Erro de leitura/escrita no disco.", Local),
        18 => ("Não foi possível criar a pasta de destino.", Local),
        19 => ("Falha ao resolver o nome do servidor (DNS).", Transitorio),
        22 => ("O servidor recusou o acesso ou respondeu de forma inesperada.", Link),
        23 => ("Redirecionamentos demais.", Link),
        24 => ("O servidor exigiu autenticação.", Link),
        28 => ("Opção inválida enviada ao motor (erro interno).", Outro),
        29 => (
            "O servidor está temporariamente indisponível ou sobrecarregado (por exemplo, HTTP 429/503).",
            Transitorio,
        ),
        32 => ("A verificação de integridade do motor falhou.", Outro),
        _ => ("O motor de download informou um erro.", Outro),
    };
    ErrorDescription { message, category }
}
