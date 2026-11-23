#!/bin/sh
set -eu

if [ "$#" -ne 2 ]; then
    echo 'Usage: compose.sh PROJECT_DIR up|pull-recreate|stop|restart' >&2
    exit 2
fi

CDPATH= cd -- "$1"

case "$2" in
    up)
        exec docker compose up -d
        ;;
    pull-recreate)
        docker compose pull
        exec docker compose up -d --force-recreate
        ;;
    stop)
        exec docker compose stop
        ;;
    restart)
        exec docker compose restart
        ;;
    *)
        printf 'Unsupported Compose operation: %s\n' "$2" >&2
        exit 2
        ;;
esac
