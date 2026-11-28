from __future__ import annotations

import os
import shutil
import tempfile
from typing import Any

from wptrunner.browsers.base import WebDriverBrowser, get_timeout_multiplier, require_arg
from wptrunner.executors import executor_kwargs as base_executor_kwargs
from wptrunner.executors.base import PytestExecutor
from wptrunner.products import Product


WPT_HOSTS = (
    "web-platform.test",
    "www.web-platform.test",
    "www1.web-platform.test",
    "www2.web-platform.test",
    "not-web-platform.test",
    "www.not-web-platform.test",
    "www1.not-web-platform.test",
    "www2.not-web-platform.test",
    "xn--lve-6lad.web-platform.test",
    "xn--lve-6lad.not-web-platform.test",
    "xn--n8j6ds53lwwkrqhv28a.web-platform.test",
    "xn--n8j6ds53lwwkrqhv28a.not-web-platform.test",
)

WPT_PORTS = (
    8000,
    8001,
    8081,
    8082,
    8093,
    8443,
    8444,
    8445,
    8446,
    8447,
    9000,
    9001,
    9444,
)


def get_product() -> Product:
    return Product(
        name="moli",
        browser_classes={None: MoliBrowser},
        check_args=check_args,
        get_browser_kwargs=browser_kwargs,
        get_executor_kwargs=executor_kwargs,
        env_options={
            "browser_host": "web-platform.test",
            "server_host": "127.0.0.1",
            "bind_address": True,
            "supports_debugger": True,
        },
        get_env_extras=env_extras,
        get_timeout_multiplier=get_timeout_multiplier,
        executor_classes={
            "wdspec": PytestExecutor,
        },
        run_info_extras=run_info_extras,
        add_arguments=add_arguments,
    )


def add_arguments(parser: Any) -> None:
    group = parser.add_argument_group("Moli-specific")
    group.add_argument(
        "--moli-arg",
        action="append",
        default=[],
        dest="moli_args",
        help="Extra argument passed to `moli serve`.",
    )
    group.add_argument(
        "--moli-no-default-host-resolve",
        action="store_true",
        default=False,
        help="Do not add default WPT .test host mappings to `moli serve`.",
    )
    group.add_argument(
        "--moli-no-image-fetch",
        action="store_true",
        default=False,
        help="Do not enable image subresource fetching for `moli serve`.",
    )


def check_args(**kwargs: Any) -> None:
    if moli_binary(kwargs) is not None:
        return
    require_arg(kwargs, "binary")


def browser_kwargs(
    logger: Any,
    test_type: str,
    run_info_data: dict[str, Any],
    config: Any,
    subsuite: Any,
    **kwargs: Any,
) -> dict[str, Any]:
    binary = moli_binary(kwargs)
    moli_args = webdriver_args(kwargs, config)
    return {
        "binary": binary,
        "webdriver_binary": binary,
        "webdriver_args": moli_args,
        "host": "127.0.0.1",
        "env": {"RUST_BACKTRACE": "1"},
        "supports_pac": False,
    }


def executor_kwargs(
    logger: Any,
    test_type: str,
    test_environment: Any,
    run_info_data: dict[str, Any],
    subsuite: Any,
    **kwargs: Any,
) -> dict[str, Any]:
    rv = base_executor_kwargs(
        test_type, test_environment, run_info_data, subsuite, **kwargs
    )
    rv["webdriver_binary"] = moli_binary(kwargs)
    rv["webdriver_args"] = webdriver_args(kwargs, test_environment.config)
    rv["capabilities"] = {
        "browserName": "moli",
        "acceptInsecureCerts": True,
        "webSocketUrl": True,
    }
    return rv


def env_extras(**kwargs: Any) -> list[Any]:
    return []


def run_info_extras(logger: Any, **kwargs: Any) -> dict[str, Any]:
    return {"browser": "moli"}


def moli_binary(kwargs: dict[str, Any]) -> str | None:
    return kwargs.get("binary") or os.environ.get("MOLI_BIN")


def webdriver_args(kwargs: dict[str, Any], config: Any | None = None) -> list[str]:
    args = list(kwargs.get("webdriver_args") or [])
    args.extend(kwargs.get("moli_args") or [])
    add_default_browser_parity_args(
        args,
        no_image_fetch=kwargs.get("moli_no_image_fetch", False),
    )
    add_default_network_args(
        args,
        no_default_host_resolve=kwargs.get(
            "moli_no_default_host_resolve", False
        ),
        config=config,
    )
    return args


def add_default_browser_parity_args(args: list[str], *, no_image_fetch: bool) -> None:
    if not any(arg in ("--layout", "--lazy", "-l") for arg in args):
        # Older Moli binaries accept this spelling.
        args.append("--layout")
    if no_image_fetch:
        for flag in (
            "--font",
            "--audio",
            "--video",
            "--media",
            "--text-track",
        ):
            append_flag_once(args, flag)
        return
    append_flag_once(args, "--resource")


def add_default_network_args(
    args: list[str], *, no_default_host_resolve: bool, config: Any | None = None
) -> None:
    append_option_once(args, "--http-no-proxy", "*")
    append_flag_once(args, "--insecure-disable-tls-host-verification")

    if no_default_host_resolve:
        return

    for host in WPT_HOSTS:
        for port in wpt_server_ports(config):
            append_option_once(args, "--http-host-resolve", f"{host}:{port}:127.0.0.1")


def wpt_server_ports(config: Any | None) -> tuple[int, ...]:
    ports = set(WPT_PORTS)
    config_ports = getattr(config, "ports", None)
    if isinstance(config_ports, dict):
        for values in config_ports.values():
            if isinstance(values, (list, tuple, set)):
                ports.update(port for port in values if isinstance(port, int))
    return tuple(sorted(ports))


def append_flag_once(args: list[str], flag: str) -> None:
    if any(arg == flag for arg in args):
        return
    args.append(flag)


def append_option_once(args: list[str], option: str, value: str) -> None:
    if option_with_value_exists(args, option, value):
        return
    args.extend([option, value])


def option_exists(args: list[str], option: str) -> bool:
    option_eq = f"{option}="
    return any(arg == option or arg.startswith(option_eq) for arg in args)


def option_with_value_exists(args: list[str], option: str, value: str) -> bool:
    option_eq = f"{option}="
    expected_eq = f"{option}={value}"
    for index, arg in enumerate(args):
        if arg == expected_eq:
            return True
        if arg == option and index + 1 < len(args) and args[index + 1] == value:
            return True
        if arg.startswith(option_eq) and option != "--http-host-resolve":
            return True
    return False


class MoliBrowser(WebDriverBrowser):
    init_timeout = 30

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        super().__init__(*args, **kwargs)
        self._default_profile_dir: str | None = None
        if not option_exists(self.webdriver_args, "--profile-dir"):
            self._default_profile_dir = tempfile.mkdtemp(prefix="moli-wpt-profile-")
            self.webdriver_args.extend(["--profile-dir", self._default_profile_dir])

    def make_command(self) -> list[str]:
        return [
            self.webdriver_binary,
            "serve",
            "--host",
            self.host,
            "--port",
            str(self.port),
            *self.webdriver_args,
        ]

    def cleanup(self) -> None:
        if self._default_profile_dir is not None:
            shutil.rmtree(self._default_profile_dir, ignore_errors=True)
            self._default_profile_dir = None
