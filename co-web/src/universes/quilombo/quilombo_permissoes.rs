use crate::quilombo_models::Papel;

/// Check if a role has a specific permission.
pub fn tem_permissao(papel: &Papel, permissao: &str) -> bool {
    match permissao {
        // Events: admin only
        "evento:criar" | "evento:editar" | "evento:excluir" => *papel == Papel::Admin,

        // Posts: admin only
        "post:criar" | "post:editar" | "post:excluir" => *papel == Papel::Admin,

        // Comments: membro+
        "comentario:criar" => matches!(papel, Papel::Membro | Papel::Editor | Papel::Admin),

        // Messages: membro+
        "mensagem:enviar" => matches!(papel, Papel::Membro | Papel::Editor | Papel::Admin),
        "mensagem:ver_contato" => *papel == Papel::Admin,

        // Missions: membro+
        "missao:criar" | "missao:editar" | "missao:excluir" => {
            matches!(papel, Papel::Membro | Papel::Editor | Papel::Admin)
        }
        "missao:aprovar_membro" => matches!(papel, Papel::Membro | Papel::Editor | Papel::Admin),

        // Profile: membro+
        "perfil:editar" => matches!(papel, Papel::Membro | Papel::Editor | Papel::Admin),

        // Admin panel
        "admin:painel" | "admin:usuarios" | "admin:atividades" => *papel == Papel::Admin,

        _ => false,
    }
}
