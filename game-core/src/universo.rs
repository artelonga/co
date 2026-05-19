use crate::engine::{Map, Universe};
use serde::{Deserialize, Serialize};

/// Trait que define o contrato de um universo no sistema Yggdrasil.
///
/// Implementado por [`UniversoLocal`] (in-memory) e futuramente por
/// implementações persistentes ou remotas.
pub trait Universo: Send + Sync {
    /// Nome canônico do universo.
    fn nome(&self) -> &str;

    /// Acesso ao mapa do universo.
    fn mapa(&self) -> &Map;

    /// Acesso mutável ao mapa.
    fn mapa_mut(&mut self) -> &mut Map;

    /// Instância do engine [`Universe`] subjacente.
    fn engine(&self) -> &Universe;
}

/// Implementação in-memory do trait [`Universo`].
///
/// Wraps [`Universe`] e adiciona a interface PT-BR do domínio Yggdrasil.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UniversoLocal {
    inner: Universe,
}

impl UniversoLocal {
    pub fn new(nome: &str, largura: usize, altura: usize) -> Self {
        Self {
            inner: Universe::new(nome.to_string(), largura, altura),
        }
    }

    pub fn from_universe(universe: Universe) -> Self {
        Self { inner: universe }
    }
}

impl Universo for UniversoLocal {
    fn nome(&self) -> &str {
        &self.inner.name
    }

    fn mapa(&self) -> &Map {
        &self.inner.map
    }

    fn mapa_mut(&mut self) -> &mut Map {
        &mut self.inner.map
    }

    fn engine(&self) -> &Universe {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn universo_local_novo_tem_nome_e_mapa() {
        let u = UniversoLocal::new("snake", 40, 20);
        assert_eq!(u.nome(), "snake");
        assert_eq!(u.mapa().width, 40);
        assert_eq!(u.mapa().height, 20);
    }

    #[test]
    fn universo_local_from_universe_preserva_nome() {
        let universe = Universe::new("tetris".to_string(), 20, 24);
        let u = UniversoLocal::from_universe(universe);
        assert_eq!(u.nome(), "tetris");
        assert_eq!(u.engine().name, "tetris");
    }

    #[test]
    fn universo_local_serializa_round_trip() {
        let u = UniversoLocal::new("lobby", 40, 20);
        let json = serde_json::to_string(&u).unwrap();
        let restored: UniversoLocal = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.nome(), "lobby");
    }
}
