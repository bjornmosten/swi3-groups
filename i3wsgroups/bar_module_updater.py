#!/usr/bin/python3

"""Listens for i3/sway workspace events and signals a bar to refresh.

Supports both polybar (via polybar-msg) and waybar (via SIGRTMIN+N).
"""

import asyncio
import shutil
import subprocess
import sys

from i3ipc import Event
from i3ipc.aio import Connection


def _detect_bar():
    """Detect which bar is running. Returns 'polybar', 'waybar', or None."""
    if shutil.which('polybar-msg'):
        return 'polybar'
    if shutil.which('waybar'):
        return 'waybar'
    return None


def _update_polybar(*_):
    subprocess.run(['polybar-msg', 'hook', 'i3-mod', '1'], check=False)


def _update_waybar(signum):
    """Send a real-time signal to waybar to refresh a custom module."""
    subprocess.run(['pkill', f'-RTMIN+{signum}', 'waybar'], check=False)


def _make_updater(bar, waybar_signal):
    if bar == 'polybar':
        return _update_polybar
    if bar == 'waybar':
        return lambda *_: _update_waybar(waybar_signal)
    print(f'error: unknown bar type: {bar}', file=sys.stderr)
    sys.exit(1)


async def run(bar=None, waybar_signal=8):
    if bar is None:
        bar = _detect_bar()
    if bar is None:
        print('error: could not detect bar (polybar or waybar). '
              'Use --bar to specify.', file=sys.stderr)
        sys.exit(1)

    updater = _make_updater(bar, waybar_signal)
    i3 = await Connection(auto_reconnect=True).connect()

    updater()
    i3.on(Event.WORKSPACE_FOCUS, updater)
    i3.on(Event.WORKSPACE_INIT, updater)
    i3.on(Event.WORKSPACE_RENAME, updater)
    i3.on(Event.WORKSPACE_MOVE, updater)
    i3.on(Event.WORKSPACE_EMPTY, updater)

    await i3.main()


def main():
    import argparse
    parser = argparse.ArgumentParser(
        description='Update bar module on workspace events (i3/sway)')
    parser.add_argument('--bar', choices=['polybar', 'waybar'],
                        help='Bar to signal (auto-detected if omitted)')
    parser.add_argument('--waybar-signal', type=int, default=8,
                        help='SIGRTMIN+N signal number for waybar (default: 8)')
    args = parser.parse_args()
    asyncio.run(run(bar=args.bar, waybar_signal=args.waybar_signal))


if __name__ == '__main__':
    main()
