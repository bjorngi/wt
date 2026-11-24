# wt shell integration for zsh. Load it with:  eval "$(wt shell-init zsh)"

typeset -g _WT_BIN=__WT_BIN__

# Runs the wt binary, then cds to the dir it asked for (via $WT_CD_FILE), if any.
wt() {
  local cd_file dir exit_code
  cd_file=$(mktemp "${TMPDIR:-/tmp}/wt-cd.XXXXXX") || return 1
  WT_CD_FILE=$cd_file "$_WT_BIN" "$@"
  exit_code=$?
  dir=$(<"$cd_file")
  command rm -f -- "$cd_file"
  if [[ -n $dir && -d $dir ]]; then
    builtin cd -- "$dir"
  fi
  return $exit_code
}

_wt_list() {
  "$_WT_BIN" __complete "$@" 2>/dev/null
}

_wt() {
  local -a items expl
  if (( CURRENT == 2 )); then
    local -a cmds=(
      'add:create a workspace, or add repos to one'
      'rm:remove a workspace, or one repo from it'
      'cleanup:remove workspaces whose remote branches are deleted'
      'ls:list workspaces'
      'cd:go to a workspace'
      'shell-init:print shell integration'
      'help:show help'
    )
    _describe -t commands 'wt command' cmds
    return
  fi

  if [[ $words[CURRENT] == -* && ( $words[2] == rm || $words[2] == cleanup ) ]]; then
    compadd -- --yes
    return
  fi

  case $words[2] in
    add)
      if (( CURRENT == 3 )); then
        items=(${(f)"$(_wt_list workspaces)"})
        _wanted workspaces expl 'feature' compadd -a items
      elif compset -P '*:'; then
        items=(${(f)"$(_wt_list branches ${IPREFIX%:})"})
        _wanted branches expl 'branch' compadd -a items
      else
        items=(${(f)"$(_wt_list specs)"})
        # Typing ':' after a repo removes the auto-inserted space, for repo:branch.
        _wanted specs expl 'repo or @preset' compadd -S ' ' -r ':' -a items
      fi
      ;;
    rm)
      if (( CURRENT == 3 )); then
        items=(${(f)"$(_wt_list workspaces)"})
        _wanted workspaces expl 'workspace' compadd -a items
      elif (( CURRENT == 4 )); then
        items=(${(f)"$(_wt_list workspace-repos $words[3])"})
        _wanted repos expl 'repo' compadd -a items
      fi
      ;;
    cd)
      if (( CURRENT == 3 )); then
        items=(${(f)"$(_wt_list workspaces)"})
        _wanted workspaces expl 'workspace' compadd -a items
      fi
      ;;
    shell-init)
      (( CURRENT == 3 )) && compadd zsh
      ;;
  esac
}

if (( $+functions[compdef] )); then
  compdef _wt wt
fi
