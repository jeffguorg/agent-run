use tracing::{debug, warn};

use crate::config::ProviderConfig;
use crate::error::AppError;
use crate::model::{
    LoadRemoteModels, ModelSpec, apply_model_api_filters, load_cached_remote_models,
    load_model_cache, log_cache_fallback, merge_models, normalize_local_models, write_model_cache,
};
use crate::protocol::{
    Protocol, base_url_for, fetch_models, model_list_url, protocol_name, resolve_key,
};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RemoteLoadMode {
    CacheOnly,
    AutoRefresh,
    ForceRefresh,
}

#[derive(Clone, Debug)]
pub struct ProviderModelCatalog {
    pub protocol: Option<Protocol>,
    pub local_models: Vec<ModelSpec>,
    pub remote_models: Vec<ModelSpec>,
    pub merged_models: Vec<ModelSpec>,
    pub used_cache: bool,
}

pub fn collect_provider_model_catalog(
    provider_name: &str,
    provider: &ProviderConfig,
    mode: RemoteLoadMode,
) -> Result<ProviderModelCatalog, AppError> {
    let local_models = normalize_local_models(&provider.models);
    let effective_filters = provider.effective_model_api_filters();
    let protocols = provider
        .protocols
        .iter()
        .copied()
        .filter(|protocol| base_url_for(provider, *protocol).is_some())
        .collect::<Vec<_>>();

    let (protocol, remote) = if !effective_filters.is_enabled() {
        (None, LoadRemoteModels::default())
    } else {
        load_remote_models_for_provider(provider_name, provider, &protocols, mode)?
    };

    let merged_models = merge_models(&local_models, &remote.models);
    debug!(
        provider_name,
        protocol = protocol.map(protocol_name),
        local_model_count = local_models.len(),
        remote_model_count = remote.models.len(),
        merged_model_count = merged_models.len(),
        used_cache = remote.used_cache,
        "built provider model catalog"
    );

    Ok(ProviderModelCatalog {
        protocol,
        local_models,
        remote_models: remote.models,
        merged_models,
        used_cache: remote.used_cache,
    })
}

pub fn load_remote_models_for_protocol(
    provider_name: &str,
    protocol: Protocol,
    base_url: &str,
    key: &str,
    force_refresh: bool,
    filters: &crate::model::ModelApiFilterConfig,
) -> Result<LoadRemoteModels, AppError> {
    if !force_refresh {
        match load_cached_remote_models(provider_name, protocol, false) {
            Ok(Some(models)) => {
                return Ok(LoadRemoteModels {
                    models,
                    used_cache: true,
                });
            }
            Ok(None) => {}
            Err(err) => {
                warn!(
                    provider_name,
                    protocol = protocol_name(protocol),
                    error = %err,
                    "failed to read model cache; continuing with remote fetch"
                );
            }
        }
    }

    match fetch_models(protocol, base_url, key) {
        Ok(models) => {
            let models = apply_model_api_filters(provider_name, &models, filters)?;
            write_model_cache(provider_name, protocol, &models)?;
            Ok(LoadRemoteModels {
                models,
                used_cache: false,
            })
        }
        Err(err) => {
            let request_url = model_list_url(protocol, base_url);
            let snapshot = match load_model_cache(provider_name) {
                Ok(snapshot) => snapshot,
                Err(cache_err) => {
                    warn!(
                        provider_name,
                        protocol = protocol_name(protocol),
                        error = %cache_err,
                        "failed to read model cache after remote fetch error"
                    );
                    None
                }
            };
            let Some(snapshot) = snapshot else {
                return Err(err);
            };
            log_cache_fallback(provider_name, protocol, &snapshot.path, &request_url, &err);
            Ok(LoadRemoteModels {
                models: snapshot.models,
                used_cache: true,
            })
        }
    }
}

fn load_remote_models_for_provider(
    provider_name: &str,
    provider: &ProviderConfig,
    protocols: &[Protocol],
    mode: RemoteLoadMode,
) -> Result<(Option<Protocol>, LoadRemoteModels), AppError> {
    let effective_filters = provider.effective_model_api_filters();
    match mode {
        RemoteLoadMode::CacheOnly => {
            let snapshot = load_model_cache(provider_name)?;
            let protocol = snapshot.as_ref().and_then(|snapshot| snapshot.protocol);
            let remote =
                snapshot.map_or_else(LoadRemoteModels::default, |snapshot| LoadRemoteModels {
                    models: snapshot.models,
                    used_cache: true,
                });
            Ok((protocol, remote))
        }
        RemoteLoadMode::AutoRefresh | RemoteLoadMode::ForceRefresh => {
            let mut best_cache: Option<(Option<Protocol>, LoadRemoteModels)> = None;
            if mode == RemoteLoadMode::AutoRefresh {
                for protocol in protocols {
                    match load_cached_remote_models(provider_name, *protocol, false) {
                        Ok(Some(models)) => {
                            consider_better_remote(
                                &mut best_cache,
                                Some(*protocol),
                                LoadRemoteModels {
                                    models,
                                    used_cache: true,
                                },
                            );
                        }
                        Ok(None) => {}
                        Err(err) => {
                            warn!(
                                provider_name,
                                protocol = protocol_name(*protocol),
                                error = %err,
                                "failed to inspect provider model cache; continuing"
                            );
                        }
                    }
                }
                if let Some(best_cache) = best_cache {
                    return Ok(best_cache);
                }
            }

            let key = resolve_key(provider)?;
            let mut last_err = None;
            let mut best_remote: Option<(Option<Protocol>, LoadRemoteModels)> = None;
            for protocol in protocols {
                let Some(base_url) = base_url_for(provider, *protocol) else {
                    continue;
                };
                match load_remote_models_for_protocol(
                    provider_name,
                    *protocol,
                    base_url,
                    &key,
                    mode == RemoteLoadMode::ForceRefresh,
                    effective_filters.as_ref(),
                ) {
                    Ok(remote) => {
                        consider_better_remote(&mut best_remote, Some(*protocol), remote);
                    }
                    Err(err) => {
                        warn!(
                            provider_name,
                            protocol = protocol_name(*protocol),
                            error = %err,
                            "failed to load remote model catalog for protocol; trying next candidate"
                        );
                        last_err = Some(err);
                    }
                }
            }
            if let Some(best_remote) = best_remote {
                return Ok(best_remote);
            }

            match load_model_cache(provider_name) {
                Ok(Some(snapshot)) => {
                    warn!(
                        provider_name,
                        protocol = snapshot.protocol.map(protocol_name),
                        path = %snapshot.path.display(),
                        "using provider model cache after all remote protocol attempts failed"
                    );
                    Ok((
                        snapshot.protocol,
                        LoadRemoteModels {
                            models: snapshot.models,
                            used_cache: true,
                        },
                    ))
                }
                Ok(None) => Err(last_err.unwrap_or_else(|| {
                    AppError::Message(format!(
                        "provider `{provider_name}` has no fetchable protocol base URL for model listing"
                    ))
                })),
                Err(err) => Err(last_err.unwrap_or(err)),
            }
        }
    }
}

fn consider_better_remote(
    best: &mut Option<(Option<Protocol>, LoadRemoteModels)>,
    protocol: Option<Protocol>,
    remote: LoadRemoteModels,
) {
    match best {
        Some((_, current))
            if model_catalog_score(&current.models) >= model_catalog_score(&remote.models) => {}
        _ => *best = Some((protocol, remote)),
    }
}

fn model_catalog_score(models: &[ModelSpec]) -> usize {
    models
        .iter()
        .map(|model| {
            usize::from(model.name.is_some()) * 4
                + usize::from(model.context_window.is_some()) * 4
                + usize::from(model.reasoning.is_some()) * 2
                + usize::from(model.vision.is_some()) * 2
        })
        .sum()
}
