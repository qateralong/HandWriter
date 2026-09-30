from __future__ import annotations

import argparse
import sys


def main() -> int:
    from handwriter.i18n import tr
    ap = argparse.ArgumentParser(description=tr("HandWriter: почерк и чертежи карандашом"))
    ap.add_argument("--browser", action="store_true",
                    help=tr("режим разработки: открыть обычную вкладку, не завершаться после закрытия окна"))
    ap.add_argument("--port", type=int, default=None, help=tr("порт (по умолчанию свободный; с --browser 8765)"))
    ap.add_argument("--no-open", action="store_true", help=tr("не открывать окно"))
    ap.add_argument("--selftest", action="store_true", help=tr("самопроверка и выход"))
    ap.add_argument("--report", default=None, help=tr("куда записать отчёт самопроверки"))
    args = ap.parse_args()

    from handwriter.logs import setup_logging
    log = setup_logging(console=True)

    if args.selftest:
        from pathlib import Path
        from handwriter.selftest import run
        if hasattr(sys.stdout, "reconfigure"):
            try:
                sys.stdout.reconfigure(encoding="utf-8")
            except Exception:
                pass
        return run(Path(args.report) if args.report else None)

    from handwriter.launcher import fatal_message, run_app
    try:
        if args.browser:
            return run_app(open_ui=not args.no_open, watchdog=False, single=False,
                           port=args.port or 8765, browser_tab=True)
        return run_app(open_ui=not args.no_open, watchdog=True, single=True, port=args.port)
    except Exception as e:
        log.exception("Ошибка запуска")
        from handwriter.paths import log_path
        fatal_message(tr(f"Программа не запустилась: {e}\n\nПодробности в журнале:\n{log_path()}"))
        return 1


if __name__ == "__main__":
    sys.exit(main())
