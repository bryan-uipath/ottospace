# A made-up agent CLI: `sleep` run under the name $1 in folder $2. $3: a process record to
# write as Claude Code does (PID, NOW filled in) into $RECORDS.
mkdir -p "$2" && cd "$2" || exit 1
[ -n "$3" ] && mkdir -p "$RECORDS" && printf '%s' "$3" | sed "s/PID/$$/; s/NOW/$(date +%s)000/" > "$RECORDS/$$.json"
exec -a "$1" sleep 600
