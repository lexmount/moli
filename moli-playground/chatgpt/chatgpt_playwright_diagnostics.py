import json
import re
from typing import Any

from chatgpt_cdp_demo import redact_sensitive_text


def redact_diagnostic_text(text: str) -> str:
    return redact_sensitive_text(text)


def is_diagnostic_url(url: str) -> bool:
    return any(
        host in url
        for host in (
            "openai.com",
            "chatgpt.com",
            "oaistatic.com",
            "sentinel.openai.com",
        )
    )


def is_conversation_diagnostic_url(url: str) -> bool:
    return any(
        marker in url
        for marker in (
            "chatgpt.com/backend-api/f/conversation",
            "chatgpt.com/backend-api/conversation/",
            "chatgpt.com/backend-api/celsius/ws/user",
            "chatgpt.com/backend-api/sentinel/",
            "ws.chatgpt.com/",
        )
    )


def set_cookie_names_from_header(value: str) -> list[str]:
    return sorted(
        {
            match.group(1)
            for match in re.finditer(
                r"(?:^|[\n,]\s*)([A-Za-z0-9_][A-Za-z0-9_.!#$%&'*+^`|~-]*)(?==)",
                value,
            )
        }
    )


def redacted_id_shape(value: Any) -> str | None:
    if not isinstance(value, str) or not value:
        return None
    match = re.search(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", value, re.I)
    if match:
        raw = match.group(0)
        return f"{raw[:8]}...{raw[-4:]}"
    if re.fullmatch(r"[A-Za-z0-9_-]{12,}", value):
        return f"{value[:6]}...{value[-4:]}"
    return f"string:{len(value)}"


def chat_request_message_shape(message: Any) -> dict[str, Any]:
    if not isinstance(message, dict):
        return {"kind": type(message).__name__}
    content = message.get("content")
    parts = content.get("parts") if isinstance(content, dict) else []
    if not isinstance(parts, list):
        parts = []
    author = message.get("author")
    metadata = message.get("metadata")
    return {
        "id": redacted_id_shape(message.get("id")),
        "role": author.get("role") if isinstance(author, dict) else "",
        "recipient": message.get("recipient") if isinstance(message.get("recipient"), str) else "",
        "contentType": content.get("content_type") if isinstance(content, dict) else "",
        "partLengths": [len(str(part)) for part in parts[:8]],
        "parent": redacted_id_shape(metadata.get("parent_id")) if isinstance(metadata, dict) else None,
        "requestId": redacted_id_shape(metadata.get("request_id")) if isinstance(metadata, dict) else None,
    }


def chat_request_post_data_shape(text: str | None) -> dict[str, Any] | None:
    if not text:
        return None
    stripped = text.strip()
    if not stripped.startswith("{"):
        return {"kind": "text", "length": len(text)}
    try:
        data = json.loads(stripped)
    except json.JSONDecodeError:
        return {"kind": "invalid-json", "length": len(text)}
    if not isinstance(data, dict):
        return {"kind": type(data).__name__, "length": len(text)}
    messages = data.get("messages")
    return {
        "kind": "json",
        "keys": sorted(str(key) for key in data.keys())[:40],
        "action": data.get("action") if isinstance(data.get("action"), str) else "",
        "conversationId": redacted_id_shape(data.get("conversation_id")),
        "parentMessageId": redacted_id_shape(data.get("parent_message_id")),
        "messages": [
            chat_request_message_shape(message)
            for message in (messages if isinstance(messages, list) else [])[:8]
        ],
        "forceParagen": data.get("force_paragen") is True,
    }


def summarize_live_trace(trace: dict[str, Any] | None) -> dict[str, Any] | None:
    if not isinstance(trace, dict):
        return None
    if "error" in trace or "message" in trace:
        return {
            "error": trace.get("error"),
            "message": trace.get("message"),
        }
    raw_events = trace.get("events")
    events = raw_events if isinstance(raw_events, list) else []
    counts: dict[str, int] = {}
    interesting_tail: list[dict[str, Any]] = []
    request_bodies: list[dict[str, Any]] = []
    for event in events:
        if not isinstance(event, dict):
            continue
        event_type = str(event.get("type") or "")
        counts[event_type] = counts.get(event_type, 0) + 1
        include_event = event_type in {
            "fetch-start",
            "fetch-response",
            "fetch-error",
            "ws-create",
            "ws-open",
            "ws-message",
            "ws-close",
            "ws-error",
            "dom-state",
            "history",
            "navigation-api",
            "app-state",
            "request-create",
            "fetch-request-body",
            "resize-observer-create",
            "resize-observer-observe",
            "resize-observer-callback",
            "intersection-observer-create",
            "intersection-observer-observe",
            "intersection-observer-callback",
            "dom-insert",
            "dom-mutation",
            "react-renderer-inject",
            "react-commit",
            "react-commit-trace-error",
            "react-devtools-inject-error",
            "react-devtools-commit-error",
        }
        if event_type in {
            "response-body-read",
            "response-clone",
            "stream-get-reader",
            "stream-read",
        } and isinstance(event.get("source"), dict):
            source_url = str(event["source"].get("url") or "")
            include_event = any(
                marker in source_url
                for marker in (
                    "chatgpt.com/backend-api/f/conversation",
                    "chatgpt.com/backend-api/conversation/",
                    "chatgpt.com/backend-api/celsius/ws/user",
                    "chatgpt.com/backend-api/sentinel/",
                )
            )
        if include_event:
            compact = {
                key: event.get(key)
                for key in (
                    "seq",
                    "t",
                    "type",
                    "url",
                    "method",
                    "status",
                    "ok",
                    "reason",
                    "data",
                    "state",
                    "stateKind",
                    "source",
                    "done",
                    "chunk",
                    "hasBody",
                    "body",
                    "count",
                    "entries",
                    "target",
                    "box",
                    "root",
                    "rootMargin",
                    "parent",
                    "child",
                    "interestingCount",
                    "records",
                    "id",
                    "version",
                    "packageName",
                    "didError",
                    "priorityLevel",
                    "visited",
                    "hostComponents",
                    "hostText",
                    "conversationHints",
                    "messageHints",
                    "turnHints",
                    "markdownHints",
                    "composerHints",
                    "dataTurnProps",
                    "dataMessageProps",
                    "dataRoleProps",
                    "textFiberLenMax",
                    "samples",
                    "name",
                    "message",
                    "op",
                    "optionsKeys",
                    "navigationType",
                    "canIntercept",
                    "hashChange",
                    "userInitiated",
                    "currentEntry",
                    "destination",
                    "from",
                    "entry",
                )
                if key in event
            }
            if isinstance(compact.get("state"), dict):
                if event_type == "app-state":
                    compact["state"] = compact_live_trace_app_state_event(compact["state"])
                else:
                    compact["state"] = compact_live_trace_state_event(compact["state"])
            if isinstance(compact.get("records"), list):
                compact["recordCount"] = len(compact["records"])
                compact["records"] = [compact_live_trace_dom_record(record) for record in compact["records"][:3]]
            if isinstance(compact.get("samples"), list):
                compact["sampleCount"] = len(compact["samples"])
                if event_type in {"dom-mutation", "dom-insert"}:
                    compact["samples"] = compact_live_trace_dom_samples(compact["samples"], limit=5)
                else:
                    compact["samples"] = compact_live_trace_react_samples(compact["samples"], limit=8)
            if event_type in {"fetch-request-body", "request-create"}:
                request_bodies.append(compact)
            interesting_tail.append(compact)
    return {
        "url": trace.get("url"),
        "snapshotErrors": trace.get("snapshotErrors"),
        "state": compact_live_trace_state(trace["state"]) if isinstance(trace.get("state"), dict) else trace.get("state"),
        "eventLoop": trace.get("eventLoop"),
        "domMutations": compact_live_trace_dom_mutations(trace.get("domMutations")),
        "reactCommits": compact_live_trace_react_commits(trace.get("reactCommits")),
        "reactConversationWrappers": compact_live_trace_conversation_wrappers(
            trace.get("reactConversationWrappers")
        ),
        "conversationMaterialization": trace.get("conversationMaterialization"),
        "conversationIdentityTrace": trace.get("conversationIdentityTrace"),
        "reactThreadFiber": compact_live_trace_thread_fiber(trace.get("reactThreadFiber")),
        "threadRendererProbes": compact_live_trace_thread_renderer_probes(
            trace.get("threadRendererProbes")
        ),
        "suspenseBoundaryProbes": compact_live_trace_suspense_boundary_probes(
            trace.get("suspenseBoundaryProbes")
        ),
        "reactionStoreProbes": compact_live_trace_reaction_store_probes(
            trace.get("reactionStoreProbes")
        ),
        "threadStoreHooks": compact_live_trace_thread_store_hooks(trace.get("threadStoreHooks")),
        "navigationApi": trace.get("navigationApi"),
        "idMapTrace": trace.get("idMapTrace"),
        "sourceStats": trace.get("sourceStats"),
        "eventCounts": counts,
        "requestBodies": request_bodies[-20:],
        "interestingTail": interesting_tail[-30:],
    }


def compact_live_trace_router_state(state: Any) -> Any:
    if not isinstance(state, dict):
        return state
    probes = state.get("loaderDataProbes")
    conversation_probe = None
    if isinstance(probes, dict):
        for key, value in probes.items():
            if "conversation" in str(key):
                conversation_probe = {
                    "route": key,
                    "kind": value.get("kind") if isinstance(value, dict) else type(value).__name__,
                    "keys": value.get("keys") if isinstance(value, dict) else None,
                    "thenType": value.get("thenType") if isinstance(value, dict) else None,
                    "mapLikeSize": value.get("mapLikeSize") if isinstance(value, dict) else None,
                }
                break
    return {
        "present": state.get("present"),
        "locationPathname": state.get("locationPathname"),
        "navigationState": state.get("navigationState"),
        "revalidation": state.get("revalidation"),
        "loaderDataKeys": state.get("loaderDataKeys"),
        "conversationProbe": conversation_probe,
        "actionDataKeys": state.get("actionDataKeys"),
        "errorKeys": state.get("errorKeys"),
        "fetcherCount": state.get("fetcherCount"),
        "blockerCount": state.get("blockerCount"),
    }


def compact_live_trace_app_state_event(state: dict[str, Any]) -> dict[str, Any]:
    query_cache = state.get("queryCache")
    query_summary = None
    if isinstance(query_cache, dict):
        query_summary = {
            "present": query_cache.get("present"),
            "queryCount": query_cache.get("queryCount"),
            "statusCounts": query_cache.get("statusCounts"),
            "fetchStatusCounts": query_cache.get("fetchStatusCounts"),
        }
    react_commits = state.get("reactCommits")
    react_summary = None
    if isinstance(react_commits, dict):
        last_commit = react_commits.get("lastCommit")
        last_summary = None
        if isinstance(last_commit, dict):
            last_summary = {
                key: last_commit.get(key)
                for key in (
                    "visited",
                    "hostComponents",
                    "hostText",
                    "conversationHints",
                    "messageHints",
                    "turnHints",
                    "markdownHints",
                    "dataTurnProps",
                    "dataMessageProps",
                    "dataRoleProps",
                )
            }
        react_summary = {
            "commitCount": react_commits.get("commitCount"),
            "commitErrors": react_commits.get("commitErrors"),
            "lastCommitAt": react_commits.get("lastCommitAt"),
            "lastCommit": last_summary,
        }
    return {
        "router": compact_live_trace_router_state(state.get("router")),
        "queryCache": query_summary,
        "serializedAppScripts": state.get("serializedAppScripts"),
        "reactCommits": react_summary,
        "activeElement": state.get("activeElement"),
    }


def compact_live_trace_state_event(state: dict[str, Any]) -> dict[str, Any]:
    selector_census = state.get("selectorCensus")
    selector_counts = {}
    if isinstance(selector_census, dict):
        for name, summary in selector_census.items():
            if isinstance(summary, dict):
                selector_counts[name] = summary.get("count")
    return {
        key: state.get(key)
        for key in (
            "url",
            "readyState",
            "assistantCount",
            "latestAssistantLen",
            "userCount",
            "latestUserLen",
            "stopButtonCount",
            "bodyTextLen",
        )
    } | {
        "selectorCounts": selector_counts,
        "appRuntime": compact_live_trace_app_state_event(state.get("appRuntime"))
        if isinstance(state.get("appRuntime"), dict)
        else None,
        "eventLoop": {
            key: state.get("eventLoop", {}).get(key)
            for key in ("timeoutFired", "intervalFired", "rafFired", "microtaskFired", "heartbeat", "now")
        }
        if isinstance(state.get("eventLoop"), dict)
        else None,
        "domMutations": compact_live_trace_dom_mutations(state.get("domMutations")),
    }


def compact_live_trace_state(state: dict[str, Any]) -> dict[str, Any]:
    selector_census = state.get("selectorCensus")
    selector_counts = {}
    if isinstance(selector_census, dict):
        for name, summary in selector_census.items():
            if isinstance(summary, dict):
                selector_counts[name] = summary.get("count")
    event_loop = state.get("eventLoop")
    compact_event_loop = None
    if isinstance(event_loop, dict):
        compact_event_loop = {
            key: event_loop.get(key)
            for key in (
                "timeoutFired",
                "intervalFired",
                "rafFired",
                "microtaskFired",
                "heartbeat",
                "now",
            )
        }
    app_runtime = state.get("appRuntime")
    compact_app_runtime = compact_live_trace_app_state(app_runtime) if isinstance(app_runtime, dict) else None
    return {
        key: state.get(key)
        for key in (
            "url",
            "readyState",
            "assistantCount",
            "latestAssistantLen",
            "userCount",
            "latestUserLen",
            "stopButtonCount",
            "bodyTextLen",
        )
    } | {
        "selectorCounts": selector_counts,
        "domTree": state.get("domTree"),
        "appRuntime": compact_app_runtime,
        "eventLoop": compact_event_loop,
        "observerApis": state.get("observerApis"),
        "domMutations": compact_live_trace_dom_mutations(state.get("domMutations")),
        "reactCommits": compact_live_trace_react_commits(state.get("reactCommits")),
    }


def compact_live_trace_app_state(state: dict[str, Any]) -> dict[str, Any]:
    return {
        "windowKeys": state.get("windowKeys"),
        "router": state.get("router"),
        "routeModules": state.get("routeModules"),
        "queryCache": state.get("queryCache"),
        "serializedAppScripts": state.get("serializedAppScripts"),
        "observerApis": state.get("observerApis"),
        "reactCommits": compact_live_trace_react_commits(state.get("reactCommits")),
        "activeElement": state.get("activeElement"),
    }


def compact_live_trace_dom_node(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    return {
        key: value.get(key)
        for key in (
            "tag",
            "id",
            "role",
            "testid",
            "dataTurn",
            "classHints",
            "textLen",
            "childCount",
        )
        if key in value
    }


def compact_live_trace_dom_record(record: Any) -> Any:
    if not isinstance(record, dict):
        return record
    added = record.get("added")
    removed = record.get("removed")
    compact = {
        key: record.get(key)
        for key in (
            "type",
            "attributeName",
        )
        if key in record
    }
    compact["target"] = compact_live_trace_dom_node(record.get("target"))
    if isinstance(added, list):
        compact["addedCount"] = len(added)
        compact["added"] = [compact_live_trace_dom_node(node) for node in added[:2]]
    if isinstance(removed, list):
        compact["removedCount"] = len(removed)
        compact["removed"] = [compact_live_trace_dom_node(node) for node in removed[:2]]
    return compact


def compact_live_trace_dom_sample(sample: Any) -> Any:
    if not isinstance(sample, dict):
        return sample
    records = sample.get("records")
    compact = {
        key: sample.get(key)
        for key in (
            "time",
            "kind",
            "count",
            "interestingCount",
        )
        if key in sample
    }
    if isinstance(records, list):
        compact["recordCount"] = len(records)
        compact["records"] = [compact_live_trace_dom_record(record) for record in records[:3]]
    return compact


def compact_live_trace_dom_samples(samples: Any, limit: int = 5) -> Any:
    if not isinstance(samples, list):
        return samples
    return [compact_live_trace_dom_sample(sample) for sample in samples[-limit:]]


def compact_live_trace_dom_mutations(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    samples = value.get("samples")
    return {
        key: value.get(key)
        for key in (
            "mutationObserverRecords",
            "mutationObserverInteresting",
            "appendChildCalls",
            "insertBeforeCalls",
            "replaceChildrenCalls",
            "interestingInserts",
            "interestingRemovals",
            "lastInterestingMutationAt",
            "now",
        )
        if key in value
    } | {
        "sampleCount": len(samples) if isinstance(samples, list) else 0,
        "samples": compact_live_trace_dom_samples(samples, limit=5),
    }


def compact_live_trace_value_shape(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    keys = value.get("keys")
    own_keys = value.get("ownKeys")
    compact = {
        key: value.get(key)
        for key in (
            "kind",
            "tag",
            "constructorName",
            "length",
            "scalar",
            "id",
            "serverId",
            "childKind",
            "textChildLen",
            "collectionKind",
            "size",
            "collectionError",
        )
        if key in value
    }
    fields = value.get("fields")
    if isinstance(fields, dict):
        compact["fields"] = dict(list(fields.items())[:16])
    if isinstance(keys, list):
        compact["keys"] = keys[:12]
    if isinstance(own_keys, list):
        compact["ownKeys"] = own_keys[:12]
    if isinstance(value.get("ctxKeys"), list):
        compact["ctxKeys"] = value["ctxKeys"][:12]
    if isinstance(value.get("configKeys"), list):
        compact["configKeys"] = value["configKeys"][:12]
    zero_arg_result = value.get("zeroArgResult")
    if isinstance(zero_arg_result, dict):
        compact["zeroArgResult"] = compact_live_trace_value_shape(zero_arg_result)
    if "zeroArgError" in value:
        compact["zeroArgError"] = value.get("zeroArgError")
    own_value_shapes = value.get("ownValueShapes")
    if isinstance(own_value_shapes, list):
        compact["ownValueShapes"] = [compact_live_trace_property_shape(item) for item in own_value_shapes[:12]]
    ctx_value_shapes = value.get("ctxOwnValueShapes")
    if isinstance(ctx_value_shapes, list):
        compact["ctxOwnValueShapes"] = [compact_live_trace_property_shape(item) for item in ctx_value_shapes[:12]]
    prototype_methods = value.get("prototypeMethods")
    if isinstance(prototype_methods, list):
        compact["prototypeMethods"] = [
            {
                "key": item.get("key"),
                "kind": item.get("kind"),
                "name": item.get("name"),
                "length": item.get("length"),
                "error": item.get("error"),
            }
            if isinstance(item, dict)
            else item
            for item in prototype_methods[:16]
        ]
    zero_arg_results = value.get("zeroArgFunctionResults")
    if isinstance(zero_arg_results, list):
        compact["zeroArgFunctionResults"] = [
            compact_live_trace_function_result_shape(item) for item in zero_arg_results[:12]
        ]
    ctx_zero_arg_results = value.get("ctxZeroArgFunctionResults")
    if isinstance(ctx_zero_arg_results, list):
        compact["ctxZeroArgFunctionResults"] = [
            compact_live_trace_function_result_shape(item) for item in ctx_zero_arg_results[:12]
        ]
    ctx_store_like = value.get("ctxStoreLike")
    if isinstance(ctx_store_like, list):
        compact["ctxStoreLike"] = [
            compact_live_trace_store_like_summary(item) for item in ctx_store_like[:8]
        ]
    thread_store = value.get("threadStore")
    if isinstance(thread_store, dict):
        compact["threadStore"] = compact_live_trace_thread_store_detail(thread_store)
    query_fields = value.get("queryFields")
    if isinstance(query_fields, dict):
        compact["queryFields"] = {
            key: compact_live_trace_value_shape(item)
            for key, item in list(query_fields.items())[:16]
        }
    for source_key in (
        "dataShape",
        "currentShape",
        "valueShape",
        "errorShape",
        "promiseShape",
    ):
        source_value = value.get(source_key)
        if isinstance(source_value, dict):
            compact[source_key] = compact_live_trace_value_shape(source_value)
    reaction_store = value.get("reactionStore")
    if isinstance(reaction_store, dict):
        compact["reactionStore"] = compact_live_trace_reaction_store_detail(reaction_store)
    conversation_detail = value.get("conversationDetail")
    if isinstance(conversation_detail, dict):
        compact["conversationDetail"] = compact_live_trace_result_detail(conversation_detail)
    selected_values = value.get("selectedValues")
    if isinstance(selected_values, dict):
        compact["selectedValues"] = {
            key: compact_live_trace_value_shape(item)
            for key, item in list(selected_values.items())[:10]
        }
    items = value.get("items")
    if isinstance(items, list):
        compact["itemCount"] = len(items)
        compact["items"] = [compact_live_trace_value_shape(item) for item in items[:3]]
    entries = value.get("entries")
    if isinstance(entries, list):
        compact["entryCount"] = len(entries)
        compact["entries"] = [
            compact_live_trace_collection_entry(item) for item in entries[:6]
        ]
    return compact


def compact_live_trace_thread_renderer_probes(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact: dict[str, Any] = {
        "rootPresent": value.get("rootPresent"),
        "visited": value.get("visited"),
        "count": value.get("count"),
    }
    probes = value.get("probes")
    if isinstance(probes, list):
        compact["probes"] = [
            compact_live_trace_thread_renderer_probe(item) for item in probes[:12]
        ]
    return compact


def compact_live_trace_thread_renderer_probe(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact = {
        key: value.get(key)
        for key in (
            "depth",
            "name",
            "tag",
            "hints",
            "sourceHint",
        )
        if key in value
    }
    props = value.get("props")
    if isinstance(props, dict):
        compact_props = {
            key: props.get(key)
            for key in (
                "keys",
                "id",
                "role",
                "testid",
                "ariaHidden",
                "inert",
                "dataTurn",
                "dataMessageId",
                "dataMessageAuthorRole",
            )
            if key in props
        }
        selected = props.get("selectedValueShapes")
        if isinstance(selected, dict):
            compact_props["selectedValueShapes"] = {
                key: compact_live_trace_value_shape(item)
                for key, item in list(selected.items())[:8]
            }
        compact["props"] = compact_props
    hooks = value.get("hooks")
    if isinstance(hooks, list):
        compact["hooks"] = [compact_live_trace_hook_state(item) for item in hooks[:16]]
    return compact


def compact_live_trace_suspense_boundary_probes(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact: dict[str, Any] = {
        "rootPresent": value.get("rootPresent"),
        "threadFiberPresent": value.get("threadFiberPresent"),
        "visited": value.get("visited"),
        "count": value.get("count"),
    }
    root_lanes = value.get("rootLanes")
    if isinstance(root_lanes, dict):
        compact["rootLanes"] = {
            key: root_lanes.get(key)
            for key in (
                "rootPresent",
                "stateNodePresent",
                "rootFiberLanes",
                "rootFiberChildLanes",
                "pendingLanes",
                "suspendedLanes",
                "pingedLanes",
                "expiredLanes",
                "errorRecoveryDisabledLanes",
                "shellSuspendCounter",
                "entangledLanes",
                "finishedLanes",
            )
            if key in root_lanes
        }
        for key in ("entanglementsShape", "hiddenUpdatesShape"):
            shape = root_lanes.get(key)
            if isinstance(shape, dict):
                compact["rootLanes"][key] = compact_live_trace_value_shape(shape)
    boundaries = value.get("boundaries")
    if isinstance(boundaries, list):
        compact["boundaries"] = [
            compact_live_trace_suspense_boundary(item) for item in boundaries[:16]
        ]
    return compact


def compact_live_trace_suspense_boundary(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact = {
        key: value.get(key)
        for key in (
            "source",
            "depth",
            "name",
            "tag",
            "tagLabel",
            "hints",
        )
        if key in value
    }
    path = value.get("path")
    if isinstance(path, list):
        compact["path"] = [
            {
                key: item.get(key)
                for key in ("name", "tag", "tagLabel")
                if isinstance(item, dict) and key in item
            }
            for item in path[-8:]
        ]
    props = value.get("props")
    if isinstance(props, dict):
        compact["props"] = compact_live_trace_suspense_child_props(props)
    internal = value.get("internal")
    if isinstance(internal, dict):
        compact["internal"] = compact_live_trace_suspense_internal(internal)
    return compact


def compact_live_trace_suspense_internal(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact = {
        key: value.get(key)
        for key in (
            "mode",
            "flags",
            "subtreeFlags",
            "lanes",
            "childLanes",
        )
        if key in value
    }
    for key in ("memoizedState", "updateQueue", "dependencies"):
        item = value.get(key)
        if isinstance(item, dict):
            compact[key] = compact_live_trace_fiber_internal_field(item)
    children = value.get("children")
    if isinstance(children, list):
        compact["children"] = [
            compact_live_trace_suspense_child(item) for item in children[:8]
        ]
    element_tree = value.get("elementTree")
    if isinstance(element_tree, dict):
        compact["elementTree"] = compact_live_trace_react_element_tree(element_tree)
    alternate = value.get("alternate")
    if isinstance(alternate, dict):
        compact["alternate"] = compact_live_trace_suspense_alternate(alternate)
    return compact


def compact_live_trace_suspense_alternate(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact = {
        key: value.get(key)
        for key in (
            "present",
            "name",
            "tag",
            "tagLabel",
            "mode",
            "flags",
            "subtreeFlags",
            "lanes",
            "childLanes",
        )
        if key in value
    }
    for key in ("memoizedState", "updateQueue", "dependencies"):
        item = value.get(key)
        if isinstance(item, dict):
            compact[key] = compact_live_trace_fiber_internal_field(item)
    children = value.get("children")
    if isinstance(children, list):
        compact["children"] = [
            compact_live_trace_suspense_child(item) for item in children[:8]
        ]
    element_tree = value.get("elementTree")
    if isinstance(element_tree, dict):
        compact["elementTree"] = compact_live_trace_react_element_tree(element_tree)
    return compact


def compact_live_trace_react_element_tree(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    kind = value.get("kind")
    if kind == "array":
        items = value.get("items")
        return {
            "kind": "array",
            "length": value.get("length"),
            "items": [
                compact_live_trace_react_element_tree(item) for item in items[:6]
            ]
            if isinstance(items, list)
            else [],
        }
    if kind == "react-element":
        compact: dict[str, Any] = {
            "kind": "react-element",
            "key": value.get("key"),
        }
        type_summary = value.get("type")
        if isinstance(type_summary, dict):
            compact["type"] = compact_live_trace_react_element_type(type_summary)
        prop_keys = value.get("propKeys")
        if isinstance(prop_keys, list):
            compact["propKeys"] = prop_keys[:12]
        selected_props = value.get("selectedProps")
        if isinstance(selected_props, dict):
            compact["selectedProps"] = {
                key: compact_live_trace_reference_shape(item)
                for key, item in list(selected_props.items())[:8]
            }
        children = value.get("children")
        if isinstance(children, dict):
            compact["children"] = compact_live_trace_react_element_tree(children)
        fallback = value.get("fallback")
        if isinstance(fallback, dict):
            compact["fallback"] = compact_live_trace_react_element_tree(fallback)
        return compact
    return compact_live_trace_value_shape(value)


def compact_live_trace_react_element_type(value: Any, depth: int = 0) -> Any:
    if not isinstance(value, dict):
        return value
    if depth >= 3:
        return {"kind": "nested"}
    compact = {
        key: value.get(key)
        for key in (
            "kind",
            "name",
            "tag",
            "keys",
            "ownKeys",
            "hasLazyInit",
            "sourceHint",
            "error",
        )
        if key in value
    }
    lazy_payload = value.get("lazyPayload")
    if isinstance(lazy_payload, dict):
        compact_payload: dict[str, Any] = {
            "status": lazy_payload.get("status"),
        }
        shape = lazy_payload.get("shape")
        if isinstance(shape, dict):
            compact_payload["shape"] = compact_live_trace_value_shape(shape)
        own_value_shapes = lazy_payload.get("ownValueShapes")
        if isinstance(own_value_shapes, list):
            compact_payload["ownValueShapes"] = [
                compact_live_trace_property_shape(item) for item in own_value_shapes[:8]
            ]
        result = lazy_payload.get("result")
        if isinstance(result, dict):
            compact_payload["result"] = compact_live_trace_value_shape(result)
        result_own_value_shapes = lazy_payload.get("resultOwnValueShapes")
        if isinstance(result_own_value_shapes, list):
            compact_payload["resultOwnValueShapes"] = [
                compact_live_trace_property_shape(item)
                for item in result_own_value_shapes[:8]
            ]
        compact["lazyPayload"] = compact_payload
    render = value.get("render")
    if isinstance(render, dict):
        compact["render"] = compact_live_trace_react_element_type(render, depth + 1)
    inner_type = value.get("innerType")
    if isinstance(inner_type, dict):
        compact["innerType"] = compact_live_trace_react_element_type(
            inner_type, depth + 1
        )
    return compact


def compact_live_trace_fiber_internal_field(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact: dict[str, Any] = {}
    for key in ("present", "error", "ownValueShapesError", "selectedValuesError"):
        if key in value:
            compact[key] = value.get(key)
    shape = value.get("shape")
    if isinstance(shape, dict):
        compact["shape"] = compact_live_trace_value_shape(shape)
    own_value_shapes = value.get("ownValueShapes")
    if isinstance(own_value_shapes, list):
        compact["ownValueShapes"] = [
            compact_live_trace_property_shape(item) for item in own_value_shapes[:10]
        ]
    selected_values = value.get("selectedValues")
    if isinstance(selected_values, dict):
        compact["selectedValues"] = {
            key: compact_live_trace_value_shape(item)
            for key, item in list(selected_values.items())[:10]
        }
    return compact


def compact_live_trace_suspense_child(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact = {
        key: value.get(key)
        for key in (
            "index",
            "name",
            "tag",
            "tagLabel",
            "hints",
            "lanes",
            "childLanes",
            "flags",
            "subtreeFlags",
        )
        if key in value
    }
    props = value.get("props")
    if isinstance(props, dict):
        compact["props"] = compact_live_trace_suspense_child_props(props)
    memoized_state = value.get("memoizedState")
    if isinstance(memoized_state, dict):
        compact["memoizedState"] = compact_live_trace_fiber_internal_field(memoized_state)
    state_node = value.get("stateNode")
    if isinstance(state_node, dict):
        compact["stateNode"] = state_node
    return compact


def compact_live_trace_suspense_child_props(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact = {
        key: value.get(key)
        for key in (
            "keys",
            "id",
            "role",
            "testid",
            "ariaHidden",
            "inert",
            "dataTurn",
            "dataMessageId",
            "dataMessageAuthorRole",
        )
        if key in value
    }
    selected = value.get("selectedValueShapes")
    if isinstance(selected, dict):
        compact["selectedValueShapes"] = {
            key: compact_live_trace_reference_shape(item)
            for key, item in list(selected.items())[:8]
        }
    return compact


def compact_live_trace_thread_store_detail(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact: dict[str, Any] = {}
    mapping = value.get("mapping")
    if isinstance(mapping, dict):
        compact["mapping"] = {
            "count": mapping.get("count"),
            "entries": mapping.get("entries")[:8] if isinstance(mapping.get("entries"), list) else [],
        }
    for key in ("mappingError", "threadCount", "threadKeys", "threadsError"):
        if key in value:
            compact[key] = value.get(key)
    return compact


def compact_live_trace_reference_shape(value: Any) -> Any:
    """Very shallow object shape for React props embedded in timeout errors."""
    if not isinstance(value, dict):
        return value
    keys = value.get("keys")
    own_keys = value.get("ownKeys")
    compact = {
        key: value.get(key)
        for key in (
            "kind",
            "tag",
            "constructorName",
            "length",
            "scalar",
            "id",
            "serverId",
            "childKind",
            "textChildLen",
            "collectionKind",
            "size",
        )
        if key in value
    }
    if isinstance(keys, list):
        compact["keys"] = keys[:8]
    if isinstance(own_keys, list):
        compact["ownKeys"] = own_keys[:8]
    if isinstance(value.get("ctxKeys"), list):
        compact["ctxKeys"] = value["ctxKeys"][:8]
    if isinstance(value.get("configKeys"), list):
        compact["configKeys"] = value["configKeys"][:8]
    zero_arg_result = value.get("zeroArgResult")
    if isinstance(zero_arg_result, dict):
        compact["zeroArgResult"] = compact_live_trace_value_shape(zero_arg_result)
    if "zeroArgError" in value:
        compact["zeroArgError"] = value.get("zeroArgError")
    items = value.get("items")
    if isinstance(items, list):
        compact["itemCount"] = len(items)
    entries = value.get("entries")
    if isinstance(entries, list):
        compact["entryCount"] = len(entries)
    if "error" in value:
        compact["error"] = value.get("error")
    return compact


def compact_live_trace_collection_entry(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact: dict[str, Any] = {}
    if "index" in value:
        compact["index"] = value.get("index")
    key_shape = value.get("key")
    if isinstance(key_shape, dict):
        compact["keyShape"] = compact_live_trace_value_shape(key_shape)
    elif key_shape is not None:
        compact["key"] = key_shape
    item_shape = value.get("value")
    if isinstance(item_shape, dict):
        compact["itemShape"] = compact_live_trace_value_shape(item_shape)
    elif item_shape is not None:
        compact["itemShape"] = item_shape
    own_shapes = value.get("valueOwnValueShapes")
    if isinstance(own_shapes, list):
        compact["itemOwnValueShapes"] = [
            compact_live_trace_property_shape(item) for item in own_shapes[:6]
        ]
    return compact


def compact_live_trace_property_shape(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact: dict[str, Any] = {"key": value.get("key")}
    if "missingDescriptor" in value:
        compact["missingDescriptor"] = value.get("missingDescriptor")
    if "error" in value:
        compact["error"] = value.get("error")
    accessor = value.get("accessor")
    if isinstance(accessor, dict):
        compact["accessor"] = {
            "get": accessor.get("get"),
            "set": accessor.get("set"),
        }
    prop_shape = value.get("shape")
    if isinstance(prop_shape, dict):
        compact["shape"] = compact_live_trace_value_shape(prop_shape)
    elif prop_shape is not None:
        compact["shape"] = prop_shape
    return compact


def compact_live_trace_function_result_shape(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact: dict[str, Any] = {
        "key": value.get("key"),
        "index": value.get("index"),
        "name": value.get("name"),
    }
    if "error" in value:
        compact["error"] = value.get("error")
    result_shape = value.get("resultShape")
    if isinstance(result_shape, dict):
        compact["resultShape"] = compact_live_trace_value_shape(result_shape)
    elif result_shape is not None:
        compact["resultShape"] = result_shape
    result_detail = value.get("resultDetail")
    if isinstance(result_detail, dict):
        compact["resultDetail"] = compact_live_trace_result_detail(result_detail)
    return compact


def compact_live_trace_store_like_summary(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact: dict[str, Any] = {"key": value.get("key")}
    for key in ("listenerCount", "error", "stateError"):
        if key in value:
            compact[key] = value.get(key)
    shape = value.get("shape")
    if isinstance(shape, dict):
        compact["shape"] = compact_live_trace_value_shape(shape)
    state_shape = value.get("stateShape")
    if isinstance(state_shape, dict):
        compact["stateShape"] = compact_live_trace_value_shape(state_shape)
    state_own_value_shapes = value.get("stateOwnValueShapes")
    if isinstance(state_own_value_shapes, list):
        compact["stateOwnValueShapes"] = [
            compact_live_trace_property_shape(item) for item in state_own_value_shapes[:12]
        ]
    selected_state_values = value.get("selectedStateValues")
    if isinstance(selected_state_values, dict):
        compact["selectedStateValues"] = {
            key: compact_live_trace_value_shape(item)
            for key, item in list(selected_state_values.items())[:12]
        }
    return compact


def compact_live_trace_result_detail(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact: dict[str, Any] = {}
    keys = value.get("keys")
    if isinstance(keys, list):
        compact["keys"] = keys[:16]
    own_keys = value.get("ownKeys")
    if isinstance(own_keys, list):
        compact["ownKeys"] = own_keys[:16]
    tree_shape = value.get("treeShape")
    if isinstance(tree_shape, dict):
        compact["treeShape"] = compact_live_trace_value_shape(tree_shape)
    if "treeCurrentLeafId" in value:
        compact["treeCurrentLeafId"] = value.get("treeCurrentLeafId")
    tree_nodes = value.get("treeNodes")
    if isinstance(tree_nodes, dict):
        compact["treeNodes"] = compact_live_trace_value_shape(tree_nodes)
    tree_display_items = value.get("treeDisplayItems")
    if isinstance(tree_display_items, dict):
        compact["treeDisplayItems"] = compact_live_trace_value_shape(tree_display_items)
    tree_display_turns = value.get("treeDisplayTurns")
    if isinstance(tree_display_turns, dict):
        compact["treeDisplayTurns"] = compact_live_trace_value_shape(tree_display_turns)
    tree_own_value_shapes = value.get("treeOwnValueShapes")
    if isinstance(tree_own_value_shapes, list):
        compact["treeOwnValueShapes"] = [
            compact_live_trace_property_shape(item) for item in tree_own_value_shapes[:12]
        ]
    tree_prototype_methods = value.get("treePrototypeMethods")
    if isinstance(tree_prototype_methods, list):
        compact["treePrototypeMethods"] = [
            {
                "key": item.get("key"),
                "kind": item.get("kind"),
                "name": item.get("name"),
                "length": item.get("length"),
                "error": item.get("error"),
            }
            if isinstance(item, dict)
            else item
            for item in tree_prototype_methods[:16]
        ]
    tree_zero_arg_results = value.get("treeZeroArgFunctionResults")
    if isinstance(tree_zero_arg_results, list):
        compact["treeZeroArgFunctionResults"] = [
            compact_live_trace_function_result_shape(item) for item in tree_zero_arg_results[:8]
        ]
    data_shape = value.get("dataShape")
    if isinstance(data_shape, dict):
        compact["dataShape"] = compact_live_trace_value_shape(data_shape)
    data_own_value_shapes = value.get("dataOwnValueShapes")
    if isinstance(data_own_value_shapes, list):
        compact["dataOwnValueShapes"] = [
            compact_live_trace_property_shape(item) for item in data_own_value_shapes[:8]
        ]
    for key in (
        "treeError",
        "treeCurrentLeafIdError",
        "treeNodesError",
        "treeDisplayItemsError",
        "treeDisplayTurnsError",
        "dataError",
    ):
        if key in value:
            compact[key] = value.get(key)
    return compact


def compact_live_trace_react_sample(sample: Any) -> Any:
    if not isinstance(sample, dict):
        return sample
    props = sample.get("props")
    compact_props: dict[str, Any] | None = None
    if isinstance(props, dict):
        compact_props = {
            key: props.get(key)
            for key in (
                "kind",
                "keys",
                "id",
                "role",
                "testid",
                "ariaHidden",
                "inert",
                "dataTurn",
                "dataMessageId",
                "dataMessageAuthorRole",
                "classHints",
                "childKind",
                "textChildLen",
                "hasDangerousHtml",
            )
            if key in props
        }
        selected_shapes = props.get("selectedValueShapes")
        if isinstance(selected_shapes, dict):
            compact_props["selectedValueShapes"] = {
                key: compact_live_trace_reference_shape(value)
                for key, value in list(selected_shapes.items())[:6]
            }
    return {
        key: sample.get(key)
        for key in (
            "name",
            "tag",
            "hints",
        )
        if key in sample
    } | {
        "props": compact_props,
        "hasStateNode": sample.get("stateNode") is not None,
    }


def compact_live_trace_conversation_wrapper(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact: dict[str, Any] = {
        "name": value.get("name"),
        "tag": value.get("tag"),
    }
    hints = value.get("hints")
    if isinstance(hints, list):
        compact["hints"] = hints[:8]
    conversation = value.get("conversation")
    if isinstance(conversation, dict):
        compact["conversation"] = compact_live_trace_value_shape(conversation)
    elif conversation is not None:
        compact["conversation"] = conversation
    subtree = value.get("subtree")
    if isinstance(subtree, dict):
        compact["subtree"] = compact_live_trace_conversation_subtree(subtree)
    for key in ("error", "message"):
        if key in value:
            compact[key] = value.get(key)
    return compact


def compact_live_trace_conversation_subtree(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact: dict[str, Any] = {
        "visited": value.get("visited"),
        "count": value.get("count"),
    }
    nodes = value.get("nodes")
    if isinstance(nodes, list):
        compact["nodes"] = [
            compact_live_trace_conversation_subtree_node(item) for item in nodes[:40]
        ]
    return compact


def compact_live_trace_conversation_subtree_node(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact = {
        key: value.get(key)
        for key in (
            "depth",
            "name",
            "tag",
            "hints",
            "sourceHint",
        )
        if key in value
    }
    props = value.get("props")
    if isinstance(props, dict):
        compact_props = {
            key: props.get(key)
            for key in (
                "keys",
                "id",
                "role",
                "testid",
                "ariaHidden",
                "inert",
                "dataTurn",
                "dataMessageId",
                "dataMessageAuthorRole",
            )
            if key in props
        }
        selected = props.get("selectedValueShapes")
        if isinstance(selected, dict):
            compact_props["selectedValueShapes"] = {
                key: compact_live_trace_reference_shape(item)
                for key, item in list(selected.items())[:8]
            }
        compact["props"] = compact_props
    hooks = value.get("hooks")
    if isinstance(hooks, list):
        compact["hooks"] = [compact_live_trace_hook_state(item) for item in hooks[:8]]
    state_node = value.get("stateNode")
    if isinstance(state_node, dict):
        compact["stateNode"] = state_node
    return compact


def compact_live_trace_hook_state(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact: dict[str, Any] = {}
    for key in (
        "index",
        "hookKeys",
        "memoizedStateError",
        "baseStateError",
        "queueError",
        "queueSnapshotError",
    ):
        if key in value:
            compact[key] = value.get(key)
    for source_key in (
        "memoizedState",
        "baseState",
        "queueValue",
        "lastRenderedState",
        "queueSnapshot",
    ):
        source_value = value.get(source_key)
        if isinstance(source_value, dict):
            compact[source_key] = compact_live_trace_value_shape(source_value)
    queue_shape = value.get("queueShape")
    if isinstance(queue_shape, dict):
        compact["queueShape"] = compact_live_trace_value_shape(queue_shape)
    return compact


def compact_live_trace_thread_store_hooks(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact: dict[str, Any] = {
        "rootPresent": value.get("rootPresent"),
        "visited": value.get("visited"),
        "count": value.get("count"),
    }
    snapshots = value.get("snapshots")
    if isinstance(snapshots, list):
        compact["snapshots"] = [
            compact_live_trace_thread_store_hook_snapshot(item) for item in snapshots[:8]
        ]
    return compact


def compact_live_trace_thread_store_hook_snapshot(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact = {
        key: value.get(key)
        for key in (
            "depth",
            "name",
            "tag",
            "hookIndex",
            "source",
        )
        if key in value
    }
    detail = value.get("detail")
    if isinstance(detail, dict):
        compact["detail"] = compact_live_trace_value_shape(detail)
    return compact


def compact_live_trace_reaction_store_probes(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact: dict[str, Any] = {
        "rootPresent": value.get("rootPresent"),
        "visited": value.get("visited"),
        "count": value.get("count"),
    }
    probes = value.get("probes")
    if isinstance(probes, list):
        compact["probes"] = [
            compact_live_trace_reaction_store_probe(item) for item in probes[:64]
        ]
    return compact


def compact_live_trace_reaction_store_probe(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact = {
        key: value.get(key)
        for key in (
            "source",
            "depth",
            "name",
            "tag",
            "tagLabel",
            "hints",
            "hookIndex",
            "hookSource",
            "selectedThreadProps",
        )
        if key in value
    }
    path = value.get("path")
    if isinstance(path, list):
        compact["path"] = [
            {
                key: item.get(key)
                for key in ("name", "tag", "tagLabel")
                if isinstance(item, dict) and key in item
            }
            for item in path[:10]
        ]
    detail = value.get("detail")
    if isinstance(detail, dict):
        compact["detail"] = compact_live_trace_reaction_store_detail(detail)
    return compact


def compact_live_trace_reaction_store_detail(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact: dict[str, Any] = {
        key: value.get(key)
        for key in (
            "kind",
            "object",
            "hasSubscribe",
            "hasGetSnapshot",
            "hasEvaluate",
            "hasOnStoreChange",
        )
        if key in value
    }
    if isinstance(value.get("ownKeys"), list):
        compact["ownKeys"] = value["ownKeys"][:12]
    for source_key in ("stateVersion", "name", "reactionShape"):
        source_value = value.get(source_key)
        if isinstance(source_value, dict):
            compact[source_key] = compact_live_trace_value_shape(source_value)
    for source_key in ("lastValue", "snapshot"):
        source_value = value.get(source_key)
        if isinstance(source_value, dict):
            compact[source_key] = compact_live_trace_value_shape(source_value)
    for error_key in (
        "stateVersionError",
        "nameError",
        "lastValueError",
        "snapshotError",
        "reactionError",
    ):
        if error_key in value:
            compact[error_key] = value.get(error_key)
    return compact


def compact_live_trace_conversation_wrappers(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact: dict[str, Any] = {
        "rootPresent": value.get("rootPresent"),
        "visited": value.get("visited"),
        "count": value.get("count"),
    }
    snapshots = value.get("snapshots")
    if isinstance(snapshots, list):
        compact["snapshots"] = [
            compact_live_trace_conversation_wrapper(item) for item in snapshots[:4]
        ]
    return compact


def compact_live_trace_thread_fiber(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    compact: dict[str, Any] = {
        "fiberPresent": value.get("fiberPresent"),
    }
    thread = value.get("thread")
    if isinstance(thread, dict):
        compact["thread"] = thread
    ancestors = value.get("ancestors")
    if isinstance(ancestors, list):
        compact["ancestors"] = [
            compact_live_trace_conversation_subtree_node(item) for item in ancestors[:36]
        ]
    subtree = value.get("subtree")
    if isinstance(subtree, dict):
        compact["subtree"] = compact_live_trace_conversation_subtree(subtree)
    return compact


def compact_live_trace_react_samples(samples: Any, limit: int = 8) -> Any:
    if not isinstance(samples, list):
        return samples
    return [compact_live_trace_react_sample(sample) for sample in samples[:limit]]


def compact_live_trace_react_commits(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    last_commit = value.get("lastCommit")
    compact_last_commit = None
    if isinstance(last_commit, dict):
        compact_last_commit = {
            key: last_commit.get(key)
            for key in (
                "id",
                "didError",
                "priorityLevel",
                "visited",
                "hostComponents",
                "hostText",
                "conversationHints",
                "messageHints",
                "turnHints",
                "markdownHints",
                "composerHints",
                "dataTurnProps",
                "dataMessageProps",
                "dataRoleProps",
                "textFiberLenMax",
            )
        }
        samples = last_commit.get("samples")
        if isinstance(samples, list):
            compact_last_commit["sampleCount"] = len(samples)
            compact_last_commit["samples"] = compact_live_trace_react_samples(samples, limit=8)
    samples = value.get("samples")
    return {
        key: value.get(key)
        for key in (
            "hookInstalled",
            "hookPreexisting",
            "rendererCount",
            "commitCount",
            "commitErrors",
            "lastCommitAt",
            "lastRendererId",
            "now",
        )
    } | {
        "lastCommit": compact_last_commit,
        "sampleCount": len(samples) if isinstance(samples, list) else 0,
        "samples": compact_live_trace_react_samples(samples[-8:] if isinstance(samples, list) else samples),
    }
