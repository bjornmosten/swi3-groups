#!/usr/bin/python3

"""Backwards-compatible wrapper — delegates to bar_module_updater."""

import asyncio

from i3wsgroups.bar_module_updater import run


def main():
    asyncio.run(run(bar='polybar'))


if __name__ == '__main__':
    main()
