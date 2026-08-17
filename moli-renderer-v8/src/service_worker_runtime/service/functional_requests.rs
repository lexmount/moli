use super::*;

impl ServiceWorkerRuntimeService {
    pub(super) fn finish_show_notification_requested(
        &self,
        request: ServiceWorkerShowNotification,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let mut state = self.inner.state.lock();
        let Some(version) = state.versions.get(&request.version_id) else {
            source_host.dispatch_show_notification_result(ServiceWorkerShowNotificationResult {
                request_id: request.request_id,
                result: Err("service worker version is unavailable".to_owned()),
            });
            return;
        };
        if version.run != run
            || version.registration_id != request.registration_id
            || version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated
        {
            source_host.dispatch_show_notification_result(ServiceWorkerShowNotificationResult {
                request_id: request.request_id,
                result: Err("service worker request is stale".to_owned()),
            });
            return;
        }
        if !matches!(
            version.running_state,
            ServiceWorkerVersionRunningState::Running { .. }
        ) {
            source_host.dispatch_show_notification_result(ServiceWorkerShowNotificationResult {
                request_id: request.request_id,
                result: Err("service worker is not running".to_owned()),
            });
            return;
        }
        let Some(registration) = state.registrations.get(&request.registration_id) else {
            source_host.dispatch_show_notification_result(ServiceWorkerShowNotificationResult {
                request_id: request.request_id,
                result: Err("service worker registration is unavailable".to_owned()),
            });
            return;
        };
        if registration.pending_unregistration
            || registration.active_version_id != Some(request.version_id)
        {
            source_host.dispatch_show_notification_result(ServiceWorkerShowNotificationResult {
                request_id: request.request_id,
                result: Err("service worker registration is unavailable".to_owned()),
            });
            return;
        }
        self.record_notification_locked(
            &mut state,
            request.registration_id,
            request.title,
            request.tag,
            request.metadata,
            request.actions,
            request.data,
        );
        source_host.dispatch_show_notification_result(ServiceWorkerShowNotificationResult {
            request_id: request.request_id,
            result: Ok(()),
        });
    }

    pub(super) fn finish_get_notifications_requested(
        &self,
        request: ServiceWorkerGetNotifications,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let delivery = {
            let state = self.inner.state.lock();
            let Some(version) = state.versions.get(&request.version_id) else {
                source_host.dispatch_get_notifications_result(
                    ServiceWorkerGetNotificationsResult {
                        request_id: request.request_id,
                        result: Err("service worker version is unavailable".to_owned()),
                    },
                );
                return;
            };
            if version.run != run {
                source_host.dispatch_get_notifications_result(
                    ServiceWorkerGetNotificationsResult {
                        request_id: request.request_id,
                        result: Err("service worker request is stale".to_owned()),
                    },
                );
                return;
            }
            let host = match &version.running_state {
                ServiceWorkerVersionRunningState::Running { host } => Some(host.clone()),
                ServiceWorkerVersionRunningState::Starting { .. }
                | ServiceWorkerVersionRunningState::Stopped => None,
            };
            let Some(host) = host else {
                source_host.dispatch_get_notifications_result(
                    ServiceWorkerGetNotificationsResult {
                        request_id: request.request_id,
                        result: Err("service worker is not running".to_owned()),
                    },
                );
                return;
            };
            let notifications = state
                .registrations
                .get(&request.registration_id)
                .filter(|registration| {
                    !registration.pending_unregistration
                        && registration.active_version_id == Some(request.version_id)
                })
                .map(|_| {
                    service_worker_notifications_for_registration_locked(
                        &state,
                        request.registration_id,
                        request.tag.as_deref(),
                    )
                })
                .unwrap_or_default();
            Some((
                host,
                ServiceWorkerGetNotificationsResult {
                    request_id: request.request_id,
                    result: Ok(notifications),
                },
            ))
        };
        if let Some((host, result)) = delivery {
            host.dispatch_get_notifications_result(result);
        }
    }

    pub(super) fn finish_sync_registration_requested(
        &self,
        request: ServiceWorkerSyncRegistration,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let (delivery, start) = {
            let mut state = self.inner.state.lock();
            let Some(version) = state.versions.get(&request.version_id) else {
                source_host.dispatch_sync_registration_result(
                    ServiceWorkerSyncRegistrationResult {
                        request_id: request.request_id,
                        result: Err("service worker version is unavailable".to_owned()),
                    },
                );
                return;
            };
            if version.run != run {
                source_host.dispatch_sync_registration_result(
                    ServiceWorkerSyncRegistrationResult {
                        request_id: request.request_id,
                        result: Err("service worker request is stale".to_owned()),
                    },
                );
                return;
            }
            let host = match &version.running_state {
                ServiceWorkerVersionRunningState::Running { host } => host.clone(),
                ServiceWorkerVersionRunningState::Starting { .. }
                | ServiceWorkerVersionRunningState::Stopped => {
                    source_host.dispatch_sync_registration_result(
                        ServiceWorkerSyncRegistrationResult {
                            request_id: request.request_id,
                            result: Err("service worker is not running".to_owned()),
                        },
                    );
                    return;
                }
            };
            let result = |message: &'static str| {
                (
                    Some((
                        host.clone(),
                        ServiceWorkerSyncRegistrationResult {
                            request_id: request.request_id,
                            result: Err(message.to_owned()),
                        },
                    )),
                    None,
                )
            };
            if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated {
                result("sync registration requires an active service worker")
            } else {
                match state.registrations.get(&request.registration_id) {
                    None => result("service worker registration is unavailable"),
                    Some(registration)
                        if registration.pending_unregistration
                            || registration.active_version_id != Some(request.version_id) =>
                    {
                        result("service worker registration is not active")
                    }
                    Some(registration) => {
                        let scope_url = registration.scope_url.clone();
                        let registration_storage_key = registration.storage_key.clone();
                        let sync_key = (request.registration_id, request.tag.clone());
                        if state
                            .sync_registrations
                            .get_mut(&sync_key)
                            .is_some_and(|record| record.mark_refire_after_finish_if_active())
                        {
                            (
                                Some((
                                    host,
                                    ServiceWorkerSyncRegistrationResult {
                                        request_id: request.request_id,
                                        result: Ok(()),
                                    },
                                )),
                                None,
                            )
                        } else {
                            let event_id = ServiceWorkerEventId(
                                self.inner.next_event_id.fetch_add(1, Ordering::Relaxed),
                            );
                            let event = ServiceWorkerSyncEvent {
                                event_id,
                                registration_id: request.registration_id,
                                version_id: request.version_id,
                                run,
                                tag: request.tag.clone(),
                                last_chance: false,
                            };
                            let start = self.start_sync_event_locked(
                                &mut state,
                                request.registration_id,
                                scope_url,
                                registration_storage_key,
                                event,
                            );
                            let accepted = !matches!(start, ServiceWorkerSyncStart::Dropped);
                            if accepted {
                                state
                                    .sync_registrations
                                    .entry(sync_key)
                                    .and_modify(|record| {
                                        record.failed_attempts = 0;
                                        record.mark_active(event_id);
                                    })
                                    .or_insert_with(|| {
                                        ServiceWorkerSyncRegistrationRecord::active(event_id)
                                    });
                            }
                            let result = if accepted {
                                Ok(())
                            } else {
                                Err("service worker sync registration could not be scheduled"
                                    .to_owned())
                            };
                            (
                                Some((
                                    host,
                                    ServiceWorkerSyncRegistrationResult {
                                        request_id: request.request_id,
                                        result,
                                    },
                                )),
                                Some(start),
                            )
                        }
                    }
                }
            }
        };
        if let Some((host, result)) = delivery {
            host.dispatch_sync_registration_result(result);
        }
        if let Some(start) = start {
            match start {
                ServiceWorkerSyncStart::Dispatch(dispatch) => {
                    let (host, event) = *dispatch;
                    self.dispatch_sync_event_to_host(host, event);
                }
                ServiceWorkerSyncStart::Start(launch) => {
                    self.start_queued_launch(*launch);
                }
                ServiceWorkerSyncStart::Queued | ServiceWorkerSyncStart::Dropped => {}
            }
        }
    }

    pub(super) fn finish_sync_get_tags_requested(
        &self,
        request: ServiceWorkerSyncGetTags,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let delivery = {
            let state = self.inner.state.lock();
            let Some(version) = state.versions.get(&request.version_id) else {
                source_host.dispatch_sync_get_tags_result(ServiceWorkerSyncGetTagsResult {
                    request_id: request.request_id,
                    result: Err("service worker version is unavailable".to_owned()),
                });
                return;
            };
            if version.run != run {
                source_host.dispatch_sync_get_tags_result(ServiceWorkerSyncGetTagsResult {
                    request_id: request.request_id,
                    result: Err("service worker request is stale".to_owned()),
                });
                return;
            }
            let host = match &version.running_state {
                ServiceWorkerVersionRunningState::Running { host } => host.clone(),
                ServiceWorkerVersionRunningState::Starting { .. }
                | ServiceWorkerVersionRunningState::Stopped => {
                    source_host.dispatch_sync_get_tags_result(ServiceWorkerSyncGetTagsResult {
                        request_id: request.request_id,
                        result: Err("service worker is not running".to_owned()),
                    });
                    return;
                }
            };
            let result = state
                .registrations
                .get(&request.registration_id)
                .filter(|registration| {
                    !registration.pending_unregistration
                        && registration.active_version_id == Some(request.version_id)
                })
                .map(|_| {
                    let mut tags: Vec<String> = state
                        .sync_registrations
                        .keys()
                        .filter(|(id, _)| *id == request.registration_id)
                        .map(|(_, tag)| tag.clone())
                        .collect();
                    tags.sort();
                    Ok(tags)
                })
                .unwrap_or_else(|| Err("service worker registration is not active".to_owned()));
            Some((
                host,
                ServiceWorkerSyncGetTagsResult {
                    request_id: request.request_id,
                    result,
                },
            ))
        };
        if let Some((host, result)) = delivery {
            host.dispatch_sync_get_tags_result(result);
        }
    }

    pub(super) fn finish_periodic_sync_registration_requested(
        &self,
        request: ServiceWorkerPeriodicSyncRegistration,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let delivery = {
            let mut state = self.inner.state.lock();
            let Some(version) = state.versions.get(&request.version_id) else {
                source_host.dispatch_periodic_sync_registration_result(
                    ServiceWorkerPeriodicSyncRegistrationResult {
                        request_id: request.request_id,
                        result: Err("service worker version is unavailable".to_owned()),
                    },
                );
                return;
            };
            if version.run != run {
                source_host.dispatch_periodic_sync_registration_result(
                    ServiceWorkerPeriodicSyncRegistrationResult {
                        request_id: request.request_id,
                        result: Err("service worker request is stale".to_owned()),
                    },
                );
                return;
            }
            let host = match &version.running_state {
                ServiceWorkerVersionRunningState::Running { host } => host.clone(),
                ServiceWorkerVersionRunningState::Starting { .. }
                | ServiceWorkerVersionRunningState::Stopped => {
                    source_host.dispatch_periodic_sync_registration_result(
                        ServiceWorkerPeriodicSyncRegistrationResult {
                            request_id: request.request_id,
                            result: Err("service worker is not running".to_owned()),
                        },
                    );
                    return;
                }
            };
            let result = if version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated
            {
                Err("periodic sync registration requires an active service worker".to_owned())
            } else {
                match state.registrations.get(&request.registration_id) {
                    Some(registration)
                        if !registration.pending_unregistration
                            && registration.active_version_id == Some(request.version_id) =>
                    {
                        state
                            .periodic_sync_registrations
                            .entry((request.registration_id, request.tag))
                            .and_modify(|record| {
                                record.update_min_interval(request.min_interval_ms);
                            })
                            .or_insert_with(|| {
                                ServiceWorkerPeriodicSyncRegistrationRecord::new(
                                    request.min_interval_ms,
                                )
                            });
                        Ok(())
                    }
                    _ => Err("service worker registration is not active".to_owned()),
                }
            };
            Some((
                host,
                ServiceWorkerPeriodicSyncRegistrationResult {
                    request_id: request.request_id,
                    result,
                },
            ))
        };
        if let Some((host, result)) = delivery {
            host.dispatch_periodic_sync_registration_result(result);
        }
    }

    pub(super) fn finish_periodic_sync_get_tags_requested(
        &self,
        request: ServiceWorkerPeriodicSyncGetTags,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let delivery = {
            let state = self.inner.state.lock();
            let Some(version) = state.versions.get(&request.version_id) else {
                source_host.dispatch_periodic_sync_get_tags_result(
                    ServiceWorkerPeriodicSyncGetTagsResult {
                        request_id: request.request_id,
                        result: Err("service worker version is unavailable".to_owned()),
                    },
                );
                return;
            };
            if version.run != run {
                source_host.dispatch_periodic_sync_get_tags_result(
                    ServiceWorkerPeriodicSyncGetTagsResult {
                        request_id: request.request_id,
                        result: Err("service worker request is stale".to_owned()),
                    },
                );
                return;
            }
            let host = match &version.running_state {
                ServiceWorkerVersionRunningState::Running { host } => host.clone(),
                ServiceWorkerVersionRunningState::Starting { .. }
                | ServiceWorkerVersionRunningState::Stopped => {
                    source_host.dispatch_periodic_sync_get_tags_result(
                        ServiceWorkerPeriodicSyncGetTagsResult {
                            request_id: request.request_id,
                            result: Err("service worker is not running".to_owned()),
                        },
                    );
                    return;
                }
            };
            let mut tags = Vec::new();
            if state
                .registrations
                .get(&request.registration_id)
                .is_some_and(|registration| {
                    !registration.pending_unregistration
                        && registration.active_version_id == Some(request.version_id)
                })
            {
                tags = state
                    .periodic_sync_registrations
                    .keys()
                    .filter(|(id, _)| *id == request.registration_id)
                    .map(|(_, tag)| tag.clone())
                    .collect();
                tags.sort();
            }
            Some((
                host,
                ServiceWorkerPeriodicSyncGetTagsResult {
                    request_id: request.request_id,
                    result: Ok(tags),
                },
            ))
        };
        if let Some((host, result)) = delivery {
            host.dispatch_periodic_sync_get_tags_result(result);
        }
    }

    pub(super) fn finish_periodic_sync_unregistration_requested(
        &self,
        request: ServiceWorkerPeriodicSyncUnregistration,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let delivery = {
            let mut state = self.inner.state.lock();
            let Some(version) = state.versions.get(&request.version_id) else {
                source_host.dispatch_periodic_sync_unregistration_result(
                    ServiceWorkerPeriodicSyncUnregistrationResult {
                        request_id: request.request_id,
                        result: Err("service worker version is unavailable".to_owned()),
                    },
                );
                return;
            };
            if version.run != run {
                source_host.dispatch_periodic_sync_unregistration_result(
                    ServiceWorkerPeriodicSyncUnregistrationResult {
                        request_id: request.request_id,
                        result: Err("service worker request is stale".to_owned()),
                    },
                );
                return;
            }
            let host = match &version.running_state {
                ServiceWorkerVersionRunningState::Running { host } => host.clone(),
                ServiceWorkerVersionRunningState::Starting { .. }
                | ServiceWorkerVersionRunningState::Stopped => {
                    source_host.dispatch_periodic_sync_unregistration_result(
                        ServiceWorkerPeriodicSyncUnregistrationResult {
                            request_id: request.request_id,
                            result: Err("service worker is not running".to_owned()),
                        },
                    );
                    return;
                }
            };
            if state
                .registrations
                .get(&request.registration_id)
                .is_some_and(|registration| {
                    !registration.pending_unregistration
                        && registration.active_version_id == Some(request.version_id)
                })
            {
                state
                    .periodic_sync_registrations
                    .remove(&(request.registration_id, request.tag));
            }
            Some((
                host,
                ServiceWorkerPeriodicSyncUnregistrationResult {
                    request_id: request.request_id,
                    result: Ok(()),
                },
            ))
        };
        if let Some((host, result)) = delivery {
            host.dispatch_periodic_sync_unregistration_result(result);
        }
    }

    pub(super) fn finish_push_subscribe_requested(
        &self,
        request: ServiceWorkerPushSubscribe,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let delivery = {
            let mut state = self.inner.state.lock();
            let Some(version) = state.versions.get(&request.version_id) else {
                source_host.dispatch_push_subscribe_result(ServiceWorkerPushSubscribeResult {
                    request_id: request.request_id,
                    result: Err("service worker version is unavailable".to_owned()),
                });
                return;
            };
            if version.run != run {
                source_host.dispatch_push_subscribe_result(ServiceWorkerPushSubscribeResult {
                    request_id: request.request_id,
                    result: Err("service worker request is stale".to_owned()),
                });
                return;
            }
            let host = match &version.running_state {
                ServiceWorkerVersionRunningState::Running { host } => host.clone(),
                ServiceWorkerVersionRunningState::Starting { .. }
                | ServiceWorkerVersionRunningState::Stopped => {
                    source_host.dispatch_push_subscribe_result(ServiceWorkerPushSubscribeResult {
                        request_id: request.request_id,
                        result: Err("service worker is not running".to_owned()),
                    });
                    return;
                }
            };
            let is_active = state
                .registrations
                .get(&request.registration_id)
                .is_some_and(|registration| {
                    !registration.pending_unregistration
                        && registration.active_version_id == Some(request.version_id)
                });
            let result = if is_active {
                let snapshot = service_worker_push_subscription_snapshot(
                    request.registration_id,
                    request.user_visible_only,
                );
                state
                    .push_subscriptions
                    .insert(request.registration_id, snapshot.clone());
                Ok(snapshot)
            } else {
                Err("service worker registration is not active".to_owned())
            };
            Some((
                host,
                ServiceWorkerPushSubscribeResult {
                    request_id: request.request_id,
                    result,
                },
            ))
        };
        if let Some((host, result)) = delivery {
            host.dispatch_push_subscribe_result(result);
        }
    }

    pub(super) fn finish_push_get_subscription_requested(
        &self,
        request: ServiceWorkerPushGetSubscription,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let delivery = {
            let state = self.inner.state.lock();
            let Some(version) = state.versions.get(&request.version_id) else {
                source_host.dispatch_push_get_subscription_result(
                    ServiceWorkerPushGetSubscriptionResult {
                        request_id: request.request_id,
                        result: Err("service worker version is unavailable".to_owned()),
                    },
                );
                return;
            };
            if version.run != run {
                source_host.dispatch_push_get_subscription_result(
                    ServiceWorkerPushGetSubscriptionResult {
                        request_id: request.request_id,
                        result: Err("service worker request is stale".to_owned()),
                    },
                );
                return;
            }
            let host = match &version.running_state {
                ServiceWorkerVersionRunningState::Running { host } => host.clone(),
                ServiceWorkerVersionRunningState::Starting { .. }
                | ServiceWorkerVersionRunningState::Stopped => {
                    source_host.dispatch_push_get_subscription_result(
                        ServiceWorkerPushGetSubscriptionResult {
                            request_id: request.request_id,
                            result: Err("service worker is not running".to_owned()),
                        },
                    );
                    return;
                }
            };
            let result = state
                .registrations
                .get(&request.registration_id)
                .filter(|registration| {
                    !registration.pending_unregistration
                        && registration.active_version_id == Some(request.version_id)
                })
                .map(|_| {
                    state
                        .push_subscriptions
                        .get(&request.registration_id)
                        .cloned()
                })
                .ok_or_else(|| "service worker registration is not active".to_owned());
            Some((
                host,
                ServiceWorkerPushGetSubscriptionResult {
                    request_id: request.request_id,
                    result,
                },
            ))
        };
        if let Some((host, result)) = delivery {
            host.dispatch_push_get_subscription_result(result);
        }
    }

    pub(super) fn finish_push_unsubscribe_requested(
        &self,
        request: ServiceWorkerPushUnsubscribe,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let delivery = {
            let mut state = self.inner.state.lock();
            let Some(version) = state.versions.get(&request.version_id) else {
                source_host.dispatch_push_unsubscribe_result(ServiceWorkerPushUnsubscribeResult {
                    request_id: request.request_id,
                    result: Err("service worker version is unavailable".to_owned()),
                });
                return;
            };
            if version.run != run {
                source_host.dispatch_push_unsubscribe_result(ServiceWorkerPushUnsubscribeResult {
                    request_id: request.request_id,
                    result: Err("service worker request is stale".to_owned()),
                });
                return;
            }
            let host = match &version.running_state {
                ServiceWorkerVersionRunningState::Running { host } => host.clone(),
                ServiceWorkerVersionRunningState::Starting { .. }
                | ServiceWorkerVersionRunningState::Stopped => {
                    source_host.dispatch_push_unsubscribe_result(
                        ServiceWorkerPushUnsubscribeResult {
                            request_id: request.request_id,
                            result: Err("service worker is not running".to_owned()),
                        },
                    );
                    return;
                }
            };
            let is_active = state
                .registrations
                .get(&request.registration_id)
                .is_some_and(|registration| {
                    !registration.pending_unregistration
                        && registration.active_version_id == Some(request.version_id)
                });
            let result = if is_active {
                Ok(state
                    .push_subscriptions
                    .remove(&request.registration_id)
                    .is_some())
            } else {
                Err("service worker registration is not active".to_owned())
            };
            Some((
                host,
                ServiceWorkerPushUnsubscribeResult {
                    request_id: request.request_id,
                    result,
                },
            ))
        };
        if let Some((host, result)) = delivery {
            host.dispatch_push_unsubscribe_result(result);
        }
    }

    pub(super) fn finish_close_notification_requested(
        &self,
        request: ServiceWorkerCloseNotification,
        run: RendererServiceWorkerRunIdentity,
    ) {
        let should_close = {
            let state = self.inner.state.lock();
            let Some(version) = state.versions.get(&request.version_id) else {
                return;
            };
            if version.run != run
                || version.registration_id != request.registration_id
                || version.lifecycle_state != ServiceWorkerVersionLifecycleState::Activated
            {
                return;
            }
            state
                .registrations
                .get(&request.registration_id)
                .is_some_and(|registration| {
                    !registration.pending_unregistration
                        && registration.active_version_id == Some(request.version_id)
                })
        };
        if should_close {
            self.close_notification(request.registration_id, request.notification_id);
        }
    }
}
