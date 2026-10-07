use std::sync::Arc;

use rivet::prelude::*;
use uuid::Uuid;

use crate::domain::product::{ListQuery, Product, SearchRequest};
use crate::state::Ctx;

/// Business logic over an immutable catalog. The catalog is loaded once at
/// startup and never mutated, so it is shared as an `Arc<[Product]>` with no lock.
#[derive(Clone)]
pub struct CatalogService {
    products: Arc<[Product]>,
}

impl CatalogService {
    pub fn load() -> Self {
        let products = seed().into();
        CatalogService { products }
    }

    pub async fn get(&self, _ctx: &Ctx, id: Uuid) -> Result<Product> {
        self.products
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| Error::not_found("product"))
    }

    pub async fn list(&self, _ctx: &Ctx, query: ListQuery) -> Result<Vec<Product>> {
        let items = self
            .products
            .iter()
            .filter(|p| query.category.as_deref().is_none_or(|c| p.category == c))
            .cloned()
            .collect();
        Ok(items)
    }

    pub async fn search(&self, _ctx: &Ctx, request: SearchRequest) -> Result<Vec<Product>> {
        if request.category.trim().is_empty() {
            return Err(Error::invalid("category must not be empty"));
        }
        let cap = request.max_price_cents.unwrap_or(u32::MAX);
        let items = self
            .products
            .iter()
            .filter(|p| p.category == request.category && p.price_cents <= cap)
            .cloned()
            .collect();
        Ok(items)
    }
}

fn seed() -> Vec<Product> {
    vec![
        Product {
            id: Uuid::from_u128(1),
            name: "Rivet Mug".into(),
            category: "merch".into(),
            price_cents: 1500,
        },
        Product {
            id: Uuid::from_u128(2),
            name: "Rivet T-Shirt".into(),
            category: "merch".into(),
            price_cents: 2500,
        },
        Product {
            id: Uuid::from_u128(3),
            name: "Support Plan".into(),
            category: "service".into(),
            price_cents: 50000,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> (CatalogService, Ctx) {
        let service = CatalogService::load();
        let state = crate::state::AppState {
            catalog: service.clone(),
        };
        (service, Ctx::detached(state))
    }

    #[tokio::test]
    async fn get_missing_returns_not_found() {
        let (service, ctx) = service();
        let err = service.get(&ctx, Uuid::from_u128(999)).await.unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    #[tokio::test]
    async fn search_rejects_empty_category() {
        let (service, ctx) = service();
        let request = SearchRequest {
            category: "  ".into(),
            max_price_cents: None,
        };
        let err = service.search(&ctx, request).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[tokio::test]
    async fn search_filters_by_price() {
        let (service, ctx) = service();
        let request = SearchRequest {
            category: "merch".into(),
            max_price_cents: Some(2000),
        };
        let found = service.search(&ctx, request).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Rivet Mug");
    }
}
