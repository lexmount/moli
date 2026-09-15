"""Parse wptserve pipe syntax for the fixture server's response helpers.

This mirrors tools/wptserve/wptserve/pipes.py and handlers.wrap_pipeline in
WPT. Recognizing a command does not implement its response transformation;
the static fixture server still only implements its documented pipe subset.
"""

from __future__ import annotations

from urllib.parse import parse_qsl


class WptPipeError(ValueError):
    """A pipe that wptserve would fail instead of serving the requested file."""


_ARGUMENT_COUNTS = {
    "header": (2, 3),
    "status": (1, 1),
    "sub": (0, 1),
    "trickle": (1, 1),
    "slice": (1, 2),
    "gzip": (0, 0),
}
_ESCAPES = {"n": "\n", "r": "\r", "t": "\t"}


def parse_pipe_commands(query: str) -> list[tuple[str, list[str]]]:
    # wrap_pipeline selects the last nonempty pipe parameter. Query decoding
    # precedes tokenization, so a percent-encoded comma is still a separator.
    pipe_string = ""
    for name, value in parse_qsl(query):
        if name == "pipe":
            pipe_string = value

    commands: list[tuple[str, list[str]]] = []
    token: list[str] = []
    in_arguments = False
    chars = iter(pipe_string)
    for char in chars:
        if in_arguments:
            if char == "\\":
                escaped = next(chars, None)
                if escaped is None:
                    raise WptPipeError("Unterminated pipe escape")
                token.append(_ESCAPES.get(escaped, escaped))
            elif char in {",", ")"}:
                commands[-1][1].append("".join(token))
                token.clear()
                in_arguments = char == ","
            else:
                # '(' and '|' are ordinary characters inside an argument.
                token.append(char)
        elif char == "(":
            commands.append(("".join(token), []))
            token.clear()
            in_arguments = True
        elif char == "|":
            if token:
                commands.append(("".join(token), []))
                token.clear()
        else:
            token.append(char)
    if in_arguments:
        # Upstream accepts an argument terminated by EOF without a final ')'.
        commands[-1][1].append("".join(token))
    elif token:
        commands.append(("".join(token), []))

    for name, args in commands:
        counts = _ARGUMENT_COUNTS.get(name)
        if counts is None:
            raise WptPipeError(f"Unknown pipe: {name!r}")
        if not counts[0] <= len(args) <= counts[1]:
            raise WptPipeError(f"Invalid argument count for {name}")
        try:
            if name == "header":
                if len(args) == 3 and args[2].lower() not in {"true", "false", "1", "0"}:
                    raise ValueError("Invalid append flag")
                # ResponseHeaders uses an isomorphic (Latin-1) encoding, after
                # the query was decoded as UTF-8. Do not silently send UTF-8
                # or let the HTTP connection abort for unencodable values.
                args[0].encode("latin-1")
                args[1].encode("latin-1")
            elif name == "status":
                int(args[0])
            elif name == "slice":
                for arg in args:
                    if arg.lower() != "null":
                        int(arg)
        except ValueError as error:
            raise WptPipeError(f"Invalid arguments for {name}") from error
    return commands
