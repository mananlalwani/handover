_handoverctl() {
    local i cur prev opts cmd
    COMPREPLY=()
    if [[ "${BASH_VERSINFO[0]}" -ge 4 ]]; then
        cur="$2"
    else
        cur="${COMP_WORDS[COMP_CWORD]}"
    fi
    prev="$3"
    cmd=""
    opts=""

    for i in "${COMP_WORDS[@]:0:COMP_CWORD}"
    do
        case "${cmd},${i}" in
            ",$1")
                cmd="handoverctl"
                ;;
            handoverctl,calls)
                cmd="handoverctl__subcmd__calls"
                ;;
            handoverctl,cancel-share)
                cmd="handoverctl__subcmd__cancel__subcmd__share"
                ;;
            handoverctl,clipboard)
                cmd="handoverctl__subcmd__clipboard"
                ;;
            handoverctl,clipboard-history)
                cmd="handoverctl__subcmd__clipboard__subcmd__history"
                ;;
            handoverctl,clipboard-mirror)
                cmd="handoverctl__subcmd__clipboard__subcmd__mirror"
                ;;
            handoverctl,contacts)
                cmd="handoverctl__subcmd__contacts"
                ;;
            handoverctl,custom)
                cmd="handoverctl__subcmd__custom"
                ;;
            handoverctl,devices)
                cmd="handoverctl__subcmd__devices"
                ;;
            handoverctl,help)
                cmd="handoverctl__subcmd__help"
                ;;
            handoverctl,media)
                cmd="handoverctl__subcmd__media"
                ;;
            handoverctl,messages)
                cmd="handoverctl__subcmd__messages"
                ;;
            handoverctl,monitor)
                cmd="handoverctl__subcmd__monitor"
                ;;
            handoverctl,native)
                cmd="handoverctl__subcmd__native"
                ;;
            handoverctl,notifications)
                cmd="handoverctl__subcmd__notifications"
                ;;
            handoverctl,notify)
                cmd="handoverctl__subcmd__notify"
                ;;
            handoverctl,screensaver)
                cmd="handoverctl__subcmd__screensaver"
                ;;
            handoverctl,send-file)
                cmd="handoverctl__subcmd__send__subcmd__file"
                ;;
            handoverctl,send-url)
                cmd="handoverctl__subcmd__send__subcmd__url"
                ;;
            handoverctl__subcmd__clipboard__subcmd__history,clear)
                cmd="handoverctl__subcmd__clipboard__subcmd__history__subcmd__clear"
                ;;
            handoverctl__subcmd__clipboard__subcmd__history,copy)
                cmd="handoverctl__subcmd__clipboard__subcmd__history__subcmd__copy"
                ;;
            handoverctl__subcmd__clipboard__subcmd__history,help)
                cmd="handoverctl__subcmd__clipboard__subcmd__history__subcmd__help"
                ;;
            handoverctl__subcmd__clipboard__subcmd__history,list)
                cmd="handoverctl__subcmd__clipboard__subcmd__history__subcmd__list"
                ;;
            handoverctl__subcmd__clipboard__subcmd__history,pin)
                cmd="handoverctl__subcmd__clipboard__subcmd__history__subcmd__pin"
                ;;
            handoverctl__subcmd__clipboard__subcmd__history,save)
                cmd="handoverctl__subcmd__clipboard__subcmd__history__subcmd__save"
                ;;
            handoverctl__subcmd__clipboard__subcmd__history,unpin)
                cmd="handoverctl__subcmd__clipboard__subcmd__history__subcmd__unpin"
                ;;
            handoverctl__subcmd__clipboard__subcmd__history__subcmd__help,clear)
                cmd="handoverctl__subcmd__clipboard__subcmd__history__subcmd__help__subcmd__clear"
                ;;
            handoverctl__subcmd__clipboard__subcmd__history__subcmd__help,copy)
                cmd="handoverctl__subcmd__clipboard__subcmd__history__subcmd__help__subcmd__copy"
                ;;
            handoverctl__subcmd__clipboard__subcmd__history__subcmd__help,help)
                cmd="handoverctl__subcmd__clipboard__subcmd__history__subcmd__help__subcmd__help"
                ;;
            handoverctl__subcmd__clipboard__subcmd__history__subcmd__help,list)
                cmd="handoverctl__subcmd__clipboard__subcmd__history__subcmd__help__subcmd__list"
                ;;
            handoverctl__subcmd__clipboard__subcmd__history__subcmd__help,pin)
                cmd="handoverctl__subcmd__clipboard__subcmd__history__subcmd__help__subcmd__pin"
                ;;
            handoverctl__subcmd__clipboard__subcmd__history__subcmd__help,save)
                cmd="handoverctl__subcmd__clipboard__subcmd__history__subcmd__help__subcmd__save"
                ;;
            handoverctl__subcmd__clipboard__subcmd__history__subcmd__help,unpin)
                cmd="handoverctl__subcmd__clipboard__subcmd__history__subcmd__help__subcmd__unpin"
                ;;
            handoverctl__subcmd__contacts,help)
                cmd="handoverctl__subcmd__contacts__subcmd__help"
                ;;
            handoverctl__subcmd__contacts,list)
                cmd="handoverctl__subcmd__contacts__subcmd__list"
                ;;
            handoverctl__subcmd__contacts,sync)
                cmd="handoverctl__subcmd__contacts__subcmd__sync"
                ;;
            handoverctl__subcmd__contacts__subcmd__help,help)
                cmd="handoverctl__subcmd__contacts__subcmd__help__subcmd__help"
                ;;
            handoverctl__subcmd__contacts__subcmd__help,list)
                cmd="handoverctl__subcmd__contacts__subcmd__help__subcmd__list"
                ;;
            handoverctl__subcmd__contacts__subcmd__help,sync)
                cmd="handoverctl__subcmd__contacts__subcmd__help__subcmd__sync"
                ;;
            handoverctl__subcmd__custom,help)
                cmd="handoverctl__subcmd__custom__subcmd__help"
                ;;
            handoverctl__subcmd__custom,list)
                cmd="handoverctl__subcmd__custom__subcmd__list"
                ;;
            handoverctl__subcmd__custom,run)
                cmd="handoverctl__subcmd__custom__subcmd__run"
                ;;
            handoverctl__subcmd__custom__subcmd__help,help)
                cmd="handoverctl__subcmd__custom__subcmd__help__subcmd__help"
                ;;
            handoverctl__subcmd__custom__subcmd__help,list)
                cmd="handoverctl__subcmd__custom__subcmd__help__subcmd__list"
                ;;
            handoverctl__subcmd__custom__subcmd__help,run)
                cmd="handoverctl__subcmd__custom__subcmd__help__subcmd__run"
                ;;
            handoverctl__subcmd__help,calls)
                cmd="handoverctl__subcmd__help__subcmd__calls"
                ;;
            handoverctl__subcmd__help,cancel-share)
                cmd="handoverctl__subcmd__help__subcmd__cancel__subcmd__share"
                ;;
            handoverctl__subcmd__help,clipboard)
                cmd="handoverctl__subcmd__help__subcmd__clipboard"
                ;;
            handoverctl__subcmd__help,clipboard-history)
                cmd="handoverctl__subcmd__help__subcmd__clipboard__subcmd__history"
                ;;
            handoverctl__subcmd__help,clipboard-mirror)
                cmd="handoverctl__subcmd__help__subcmd__clipboard__subcmd__mirror"
                ;;
            handoverctl__subcmd__help,contacts)
                cmd="handoverctl__subcmd__help__subcmd__contacts"
                ;;
            handoverctl__subcmd__help,custom)
                cmd="handoverctl__subcmd__help__subcmd__custom"
                ;;
            handoverctl__subcmd__help,devices)
                cmd="handoverctl__subcmd__help__subcmd__devices"
                ;;
            handoverctl__subcmd__help,help)
                cmd="handoverctl__subcmd__help__subcmd__help"
                ;;
            handoverctl__subcmd__help,media)
                cmd="handoverctl__subcmd__help__subcmd__media"
                ;;
            handoverctl__subcmd__help,messages)
                cmd="handoverctl__subcmd__help__subcmd__messages"
                ;;
            handoverctl__subcmd__help,monitor)
                cmd="handoverctl__subcmd__help__subcmd__monitor"
                ;;
            handoverctl__subcmd__help,native)
                cmd="handoverctl__subcmd__help__subcmd__native"
                ;;
            handoverctl__subcmd__help,notifications)
                cmd="handoverctl__subcmd__help__subcmd__notifications"
                ;;
            handoverctl__subcmd__help,notify)
                cmd="handoverctl__subcmd__help__subcmd__notify"
                ;;
            handoverctl__subcmd__help,screensaver)
                cmd="handoverctl__subcmd__help__subcmd__screensaver"
                ;;
            handoverctl__subcmd__help,send-file)
                cmd="handoverctl__subcmd__help__subcmd__send__subcmd__file"
                ;;
            handoverctl__subcmd__help,send-url)
                cmd="handoverctl__subcmd__help__subcmd__send__subcmd__url"
                ;;
            handoverctl__subcmd__help__subcmd__clipboard__subcmd__history,clear)
                cmd="handoverctl__subcmd__help__subcmd__clipboard__subcmd__history__subcmd__clear"
                ;;
            handoverctl__subcmd__help__subcmd__clipboard__subcmd__history,copy)
                cmd="handoverctl__subcmd__help__subcmd__clipboard__subcmd__history__subcmd__copy"
                ;;
            handoverctl__subcmd__help__subcmd__clipboard__subcmd__history,list)
                cmd="handoverctl__subcmd__help__subcmd__clipboard__subcmd__history__subcmd__list"
                ;;
            handoverctl__subcmd__help__subcmd__clipboard__subcmd__history,pin)
                cmd="handoverctl__subcmd__help__subcmd__clipboard__subcmd__history__subcmd__pin"
                ;;
            handoverctl__subcmd__help__subcmd__clipboard__subcmd__history,save)
                cmd="handoverctl__subcmd__help__subcmd__clipboard__subcmd__history__subcmd__save"
                ;;
            handoverctl__subcmd__help__subcmd__clipboard__subcmd__history,unpin)
                cmd="handoverctl__subcmd__help__subcmd__clipboard__subcmd__history__subcmd__unpin"
                ;;
            handoverctl__subcmd__help__subcmd__contacts,list)
                cmd="handoverctl__subcmd__help__subcmd__contacts__subcmd__list"
                ;;
            handoverctl__subcmd__help__subcmd__contacts,sync)
                cmd="handoverctl__subcmd__help__subcmd__contacts__subcmd__sync"
                ;;
            handoverctl__subcmd__help__subcmd__custom,list)
                cmd="handoverctl__subcmd__help__subcmd__custom__subcmd__list"
                ;;
            handoverctl__subcmd__help__subcmd__custom,run)
                cmd="handoverctl__subcmd__help__subcmd__custom__subcmd__run"
                ;;
            handoverctl__subcmd__help__subcmd__media,next)
                cmd="handoverctl__subcmd__help__subcmd__media__subcmd__next"
                ;;
            handoverctl__subcmd__help__subcmd__media,pause)
                cmd="handoverctl__subcmd__help__subcmd__media__subcmd__pause"
                ;;
            handoverctl__subcmd__help__subcmd__media,play)
                cmd="handoverctl__subcmd__help__subcmd__media__subcmd__play"
                ;;
            handoverctl__subcmd__help__subcmd__media,play-pause)
                cmd="handoverctl__subcmd__help__subcmd__media__subcmd__play__subcmd__pause"
                ;;
            handoverctl__subcmd__help__subcmd__media,previous)
                cmd="handoverctl__subcmd__help__subcmd__media__subcmd__previous"
                ;;
            handoverctl__subcmd__help__subcmd__messages,accounts)
                cmd="handoverctl__subcmd__help__subcmd__messages__subcmd__accounts"
                ;;
            handoverctl__subcmd__help__subcmd__messages,conversations)
                cmd="handoverctl__subcmd__help__subcmd__messages__subcmd__conversations"
                ;;
            handoverctl__subcmd__help__subcmd__messages,delete)
                cmd="handoverctl__subcmd__help__subcmd__messages__subcmd__delete"
                ;;
            handoverctl__subcmd__help__subcmd__messages,history)
                cmd="handoverctl__subcmd__help__subcmd__messages__subcmd__history"
                ;;
            handoverctl__subcmd__help__subcmd__messages,login)
                cmd="handoverctl__subcmd__help__subcmd__messages__subcmd__login"
                ;;
            handoverctl__subcmd__help__subcmd__messages,logout)
                cmd="handoverctl__subcmd__help__subcmd__messages__subcmd__logout"
                ;;
            handoverctl__subcmd__help__subcmd__messages,open)
                cmd="handoverctl__subcmd__help__subcmd__messages__subcmd__open"
                ;;
            handoverctl__subcmd__help__subcmd__messages,react)
                cmd="handoverctl__subcmd__help__subcmd__messages__subcmd__react"
                ;;
            handoverctl__subcmd__help__subcmd__messages,read)
                cmd="handoverctl__subcmd__help__subcmd__messages__subcmd__read"
                ;;
            handoverctl__subcmd__help__subcmd__messages,reply)
                cmd="handoverctl__subcmd__help__subcmd__messages__subcmd__reply"
                ;;
            handoverctl__subcmd__help__subcmd__messages,send)
                cmd="handoverctl__subcmd__help__subcmd__messages__subcmd__send"
                ;;
            handoverctl__subcmd__help__subcmd__messages,send-file)
                cmd="handoverctl__subcmd__help__subcmd__messages__subcmd__send__subcmd__file"
                ;;
            handoverctl__subcmd__help__subcmd__messages,sync)
                cmd="handoverctl__subcmd__help__subcmd__messages__subcmd__sync"
                ;;
            handoverctl__subcmd__help__subcmd__messages,typing)
                cmd="handoverctl__subcmd__help__subcmd__messages__subcmd__typing"
                ;;
            handoverctl__subcmd__help__subcmd__messages,unreact)
                cmd="handoverctl__subcmd__help__subcmd__messages__subcmd__unreact"
                ;;
            handoverctl__subcmd__help__subcmd__native,call)
                cmd="handoverctl__subcmd__help__subcmd__native__subcmd__call"
                ;;
            handoverctl__subcmd__help__subcmd__native,filesystem-list)
                cmd="handoverctl__subcmd__help__subcmd__native__subcmd__filesystem__subcmd__list"
                ;;
            handoverctl__subcmd__help__subcmd__native,keep-awake)
                cmd="handoverctl__subcmd__help__subcmd__native__subcmd__keep__subcmd__awake"
                ;;
            handoverctl__subcmd__help__subcmd__native,lock)
                cmd="handoverctl__subcmd__help__subcmd__native__subcmd__lock"
                ;;
            handoverctl__subcmd__help__subcmd__native,pair)
                cmd="handoverctl__subcmd__help__subcmd__native__subcmd__pair"
                ;;
            handoverctl__subcmd__help__subcmd__native,peers)
                cmd="handoverctl__subcmd__help__subcmd__native__subcmd__peers"
                ;;
            handoverctl__subcmd__help__subcmd__native,pending)
                cmd="handoverctl__subcmd__help__subcmd__native__subcmd__pending"
                ;;
            handoverctl__subcmd__help__subcmd__native,ping)
                cmd="handoverctl__subcmd__help__subcmd__native__subcmd__ping"
                ;;
            handoverctl__subcmd__help__subcmd__native,ring)
                cmd="handoverctl__subcmd__help__subcmd__native__subcmd__ring"
                ;;
            handoverctl__subcmd__help__subcmd__native,tethering)
                cmd="handoverctl__subcmd__help__subcmd__native__subcmd__tethering"
                ;;
            handoverctl__subcmd__help__subcmd__native,unpair)
                cmd="handoverctl__subcmd__help__subcmd__native__subcmd__unpair"
                ;;
            handoverctl__subcmd__media,help)
                cmd="handoverctl__subcmd__media__subcmd__help"
                ;;
            handoverctl__subcmd__media,next)
                cmd="handoverctl__subcmd__media__subcmd__next"
                ;;
            handoverctl__subcmd__media,pause)
                cmd="handoverctl__subcmd__media__subcmd__pause"
                ;;
            handoverctl__subcmd__media,play)
                cmd="handoverctl__subcmd__media__subcmd__play"
                ;;
            handoverctl__subcmd__media,play-pause)
                cmd="handoverctl__subcmd__media__subcmd__play__subcmd__pause"
                ;;
            handoverctl__subcmd__media,previous)
                cmd="handoverctl__subcmd__media__subcmd__previous"
                ;;
            handoverctl__subcmd__media__subcmd__help,help)
                cmd="handoverctl__subcmd__media__subcmd__help__subcmd__help"
                ;;
            handoverctl__subcmd__media__subcmd__help,next)
                cmd="handoverctl__subcmd__media__subcmd__help__subcmd__next"
                ;;
            handoverctl__subcmd__media__subcmd__help,pause)
                cmd="handoverctl__subcmd__media__subcmd__help__subcmd__pause"
                ;;
            handoverctl__subcmd__media__subcmd__help,play)
                cmd="handoverctl__subcmd__media__subcmd__help__subcmd__play"
                ;;
            handoverctl__subcmd__media__subcmd__help,play-pause)
                cmd="handoverctl__subcmd__media__subcmd__help__subcmd__play__subcmd__pause"
                ;;
            handoverctl__subcmd__media__subcmd__help,previous)
                cmd="handoverctl__subcmd__media__subcmd__help__subcmd__previous"
                ;;
            handoverctl__subcmd__messages,accounts)
                cmd="handoverctl__subcmd__messages__subcmd__accounts"
                ;;
            handoverctl__subcmd__messages,conversations)
                cmd="handoverctl__subcmd__messages__subcmd__conversations"
                ;;
            handoverctl__subcmd__messages,delete)
                cmd="handoverctl__subcmd__messages__subcmd__delete"
                ;;
            handoverctl__subcmd__messages,help)
                cmd="handoverctl__subcmd__messages__subcmd__help"
                ;;
            handoverctl__subcmd__messages,history)
                cmd="handoverctl__subcmd__messages__subcmd__history"
                ;;
            handoverctl__subcmd__messages,login)
                cmd="handoverctl__subcmd__messages__subcmd__login"
                ;;
            handoverctl__subcmd__messages,logout)
                cmd="handoverctl__subcmd__messages__subcmd__logout"
                ;;
            handoverctl__subcmd__messages,open)
                cmd="handoverctl__subcmd__messages__subcmd__open"
                ;;
            handoverctl__subcmd__messages,react)
                cmd="handoverctl__subcmd__messages__subcmd__react"
                ;;
            handoverctl__subcmd__messages,read)
                cmd="handoverctl__subcmd__messages__subcmd__read"
                ;;
            handoverctl__subcmd__messages,reply)
                cmd="handoverctl__subcmd__messages__subcmd__reply"
                ;;
            handoverctl__subcmd__messages,send)
                cmd="handoverctl__subcmd__messages__subcmd__send"
                ;;
            handoverctl__subcmd__messages,send-file)
                cmd="handoverctl__subcmd__messages__subcmd__send__subcmd__file"
                ;;
            handoverctl__subcmd__messages,sync)
                cmd="handoverctl__subcmd__messages__subcmd__sync"
                ;;
            handoverctl__subcmd__messages,typing)
                cmd="handoverctl__subcmd__messages__subcmd__typing"
                ;;
            handoverctl__subcmd__messages,unreact)
                cmd="handoverctl__subcmd__messages__subcmd__unreact"
                ;;
            handoverctl__subcmd__messages__subcmd__help,accounts)
                cmd="handoverctl__subcmd__messages__subcmd__help__subcmd__accounts"
                ;;
            handoverctl__subcmd__messages__subcmd__help,conversations)
                cmd="handoverctl__subcmd__messages__subcmd__help__subcmd__conversations"
                ;;
            handoverctl__subcmd__messages__subcmd__help,delete)
                cmd="handoverctl__subcmd__messages__subcmd__help__subcmd__delete"
                ;;
            handoverctl__subcmd__messages__subcmd__help,help)
                cmd="handoverctl__subcmd__messages__subcmd__help__subcmd__help"
                ;;
            handoverctl__subcmd__messages__subcmd__help,history)
                cmd="handoverctl__subcmd__messages__subcmd__help__subcmd__history"
                ;;
            handoverctl__subcmd__messages__subcmd__help,login)
                cmd="handoverctl__subcmd__messages__subcmd__help__subcmd__login"
                ;;
            handoverctl__subcmd__messages__subcmd__help,logout)
                cmd="handoverctl__subcmd__messages__subcmd__help__subcmd__logout"
                ;;
            handoverctl__subcmd__messages__subcmd__help,open)
                cmd="handoverctl__subcmd__messages__subcmd__help__subcmd__open"
                ;;
            handoverctl__subcmd__messages__subcmd__help,react)
                cmd="handoverctl__subcmd__messages__subcmd__help__subcmd__react"
                ;;
            handoverctl__subcmd__messages__subcmd__help,read)
                cmd="handoverctl__subcmd__messages__subcmd__help__subcmd__read"
                ;;
            handoverctl__subcmd__messages__subcmd__help,reply)
                cmd="handoverctl__subcmd__messages__subcmd__help__subcmd__reply"
                ;;
            handoverctl__subcmd__messages__subcmd__help,send)
                cmd="handoverctl__subcmd__messages__subcmd__help__subcmd__send"
                ;;
            handoverctl__subcmd__messages__subcmd__help,send-file)
                cmd="handoverctl__subcmd__messages__subcmd__help__subcmd__send__subcmd__file"
                ;;
            handoverctl__subcmd__messages__subcmd__help,sync)
                cmd="handoverctl__subcmd__messages__subcmd__help__subcmd__sync"
                ;;
            handoverctl__subcmd__messages__subcmd__help,typing)
                cmd="handoverctl__subcmd__messages__subcmd__help__subcmd__typing"
                ;;
            handoverctl__subcmd__messages__subcmd__help,unreact)
                cmd="handoverctl__subcmd__messages__subcmd__help__subcmd__unreact"
                ;;
            handoverctl__subcmd__native,call)
                cmd="handoverctl__subcmd__native__subcmd__call"
                ;;
            handoverctl__subcmd__native,filesystem-list)
                cmd="handoverctl__subcmd__native__subcmd__filesystem__subcmd__list"
                ;;
            handoverctl__subcmd__native,help)
                cmd="handoverctl__subcmd__native__subcmd__help"
                ;;
            handoverctl__subcmd__native,keep-awake)
                cmd="handoverctl__subcmd__native__subcmd__keep__subcmd__awake"
                ;;
            handoverctl__subcmd__native,lock)
                cmd="handoverctl__subcmd__native__subcmd__lock"
                ;;
            handoverctl__subcmd__native,pair)
                cmd="handoverctl__subcmd__native__subcmd__pair"
                ;;
            handoverctl__subcmd__native,peers)
                cmd="handoverctl__subcmd__native__subcmd__peers"
                ;;
            handoverctl__subcmd__native,pending)
                cmd="handoverctl__subcmd__native__subcmd__pending"
                ;;
            handoverctl__subcmd__native,ping)
                cmd="handoverctl__subcmd__native__subcmd__ping"
                ;;
            handoverctl__subcmd__native,ring)
                cmd="handoverctl__subcmd__native__subcmd__ring"
                ;;
            handoverctl__subcmd__native,tethering)
                cmd="handoverctl__subcmd__native__subcmd__tethering"
                ;;
            handoverctl__subcmd__native,unpair)
                cmd="handoverctl__subcmd__native__subcmd__unpair"
                ;;
            handoverctl__subcmd__native__subcmd__help,call)
                cmd="handoverctl__subcmd__native__subcmd__help__subcmd__call"
                ;;
            handoverctl__subcmd__native__subcmd__help,filesystem-list)
                cmd="handoverctl__subcmd__native__subcmd__help__subcmd__filesystem__subcmd__list"
                ;;
            handoverctl__subcmd__native__subcmd__help,help)
                cmd="handoverctl__subcmd__native__subcmd__help__subcmd__help"
                ;;
            handoverctl__subcmd__native__subcmd__help,keep-awake)
                cmd="handoverctl__subcmd__native__subcmd__help__subcmd__keep__subcmd__awake"
                ;;
            handoverctl__subcmd__native__subcmd__help,lock)
                cmd="handoverctl__subcmd__native__subcmd__help__subcmd__lock"
                ;;
            handoverctl__subcmd__native__subcmd__help,pair)
                cmd="handoverctl__subcmd__native__subcmd__help__subcmd__pair"
                ;;
            handoverctl__subcmd__native__subcmd__help,peers)
                cmd="handoverctl__subcmd__native__subcmd__help__subcmd__peers"
                ;;
            handoverctl__subcmd__native__subcmd__help,pending)
                cmd="handoverctl__subcmd__native__subcmd__help__subcmd__pending"
                ;;
            handoverctl__subcmd__native__subcmd__help,ping)
                cmd="handoverctl__subcmd__native__subcmd__help__subcmd__ping"
                ;;
            handoverctl__subcmd__native__subcmd__help,ring)
                cmd="handoverctl__subcmd__native__subcmd__help__subcmd__ring"
                ;;
            handoverctl__subcmd__native__subcmd__help,tethering)
                cmd="handoverctl__subcmd__native__subcmd__help__subcmd__tethering"
                ;;
            handoverctl__subcmd__native__subcmd__help,unpair)
                cmd="handoverctl__subcmd__native__subcmd__help__subcmd__unpair"
                ;;
            *)
                ;;
        esac
    done

    case "${cmd}" in
        handoverctl)
            opts="-h -V --help --version devices native notifications contacts media monitor send-url notify clipboard send-file screensaver clipboard-mirror clipboard-history cancel-share custom calls messages help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 1 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__calls)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__cancel__subcmd__share)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__clipboard)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__clipboard__subcmd__history)
            opts="-h --help list save pin unpin copy clear help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__clipboard__subcmd__history__subcmd__clear)
            opts="-h --all --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__clipboard__subcmd__history__subcmd__copy)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__clipboard__subcmd__history__subcmd__help)
            opts="list save pin unpin copy clear help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__clipboard__subcmd__history__subcmd__help__subcmd__clear)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__clipboard__subcmd__history__subcmd__help__subcmd__copy)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__clipboard__subcmd__history__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__clipboard__subcmd__history__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__clipboard__subcmd__history__subcmd__help__subcmd__pin)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__clipboard__subcmd__history__subcmd__help__subcmd__save)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__clipboard__subcmd__history__subcmd__help__subcmd__unpin)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__clipboard__subcmd__history__subcmd__list)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__clipboard__subcmd__history__subcmd__pin)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__clipboard__subcmd__history__subcmd__save)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__clipboard__subcmd__history__subcmd__unpin)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__clipboard__subcmd__mirror)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__contacts)
            opts="-h --help list sync help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__contacts__subcmd__help)
            opts="list sync help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__contacts__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__contacts__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__contacts__subcmd__help__subcmd__sync)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__contacts__subcmd__list)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__contacts__subcmd__sync)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__custom)
            opts="-h --help list run help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__custom__subcmd__help)
            opts="list run help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__custom__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__custom__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__custom__subcmd__help__subcmd__run)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__custom__subcmd__list)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__custom__subcmd__run)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__devices)
            opts="-h --include-compatibility --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help)
            opts="devices native notifications contacts media monitor send-url notify clipboard send-file screensaver clipboard-mirror clipboard-history cancel-share custom calls messages help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__calls)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__cancel__subcmd__share)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__clipboard)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__clipboard__subcmd__history)
            opts="list save pin unpin copy clear"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__clipboard__subcmd__history__subcmd__clear)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__clipboard__subcmd__history__subcmd__copy)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__clipboard__subcmd__history__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__clipboard__subcmd__history__subcmd__pin)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__clipboard__subcmd__history__subcmd__save)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__clipboard__subcmd__history__subcmd__unpin)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__clipboard__subcmd__mirror)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__contacts)
            opts="list sync"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__contacts__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__contacts__subcmd__sync)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__custom)
            opts="list run"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__custom__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__custom__subcmd__run)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__devices)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__media)
            opts="play pause play-pause next previous"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__media__subcmd__next)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__media__subcmd__pause)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__media__subcmd__play)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__media__subcmd__play__subcmd__pause)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__media__subcmd__previous)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__messages)
            opts="accounts conversations history send send-file reply react unreact read typing delete open login logout sync"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__messages__subcmd__accounts)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__messages__subcmd__conversations)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__messages__subcmd__delete)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__messages__subcmd__history)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__messages__subcmd__login)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__messages__subcmd__logout)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__messages__subcmd__open)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__messages__subcmd__react)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__messages__subcmd__read)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__messages__subcmd__reply)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__messages__subcmd__send)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__messages__subcmd__send__subcmd__file)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__messages__subcmd__sync)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__messages__subcmd__typing)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__messages__subcmd__unreact)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__monitor)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__native)
            opts="peers pending pair unpair ping ring lock keep-awake tethering filesystem-list call"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__native__subcmd__call)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__native__subcmd__filesystem__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__native__subcmd__keep__subcmd__awake)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__native__subcmd__lock)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__native__subcmd__pair)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__native__subcmd__peers)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__native__subcmd__pending)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__native__subcmd__ping)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__native__subcmd__ring)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__native__subcmd__tethering)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__native__subcmd__unpair)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__notifications)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__notify)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__screensaver)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__send__subcmd__file)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__help__subcmd__send__subcmd__url)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__media)
            opts="-h --help play pause play-pause next previous help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__media__subcmd__help)
            opts="play pause play-pause next previous help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__media__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__media__subcmd__help__subcmd__next)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__media__subcmd__help__subcmd__pause)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__media__subcmd__help__subcmd__play)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__media__subcmd__help__subcmd__play__subcmd__pause)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__media__subcmd__help__subcmd__previous)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__media__subcmd__next)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__media__subcmd__pause)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__media__subcmd__play)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__media__subcmd__play__subcmd__pause)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__media__subcmd__previous)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages)
            opts="-h --help accounts conversations history send send-file reply react unreact read typing delete open login logout sync help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__accounts)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__conversations)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__delete)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__help)
            opts="accounts conversations history send send-file reply react unreact read typing delete open login logout sync help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__help__subcmd__accounts)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__help__subcmd__conversations)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__help__subcmd__delete)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__help__subcmd__history)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__help__subcmd__login)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__help__subcmd__logout)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__help__subcmd__open)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__help__subcmd__react)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__help__subcmd__read)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__help__subcmd__reply)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__help__subcmd__send)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__help__subcmd__send__subcmd__file)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__help__subcmd__sync)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__help__subcmd__typing)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__help__subcmd__unreact)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__history)
            opts="-h --limit --cursor --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --cursor)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__login)
            opts="-h --from-file --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --from-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__logout)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__open)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__react)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__read)
            opts="-h --message --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__reply)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__send)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__send__subcmd__file)
            opts="-h --caption --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --caption)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__sync)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__typing)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__messages__subcmd__unreact)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__monitor)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native)
            opts="-h --help peers pending pair unpair ping ring lock keep-awake tethering filesystem-list call help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__call)
            opts="-h --confirm --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__filesystem__subcmd__list)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__help)
            opts="peers pending pair unpair ping ring lock keep-awake tethering filesystem-list call help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__help__subcmd__call)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__help__subcmd__filesystem__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__help__subcmd__keep__subcmd__awake)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__help__subcmd__lock)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__help__subcmd__pair)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__help__subcmd__peers)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__help__subcmd__pending)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__help__subcmd__ping)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__help__subcmd__ring)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__help__subcmd__tethering)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__help__subcmd__unpair)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__keep__subcmd__awake)
            opts="-h --release --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__lock)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__pair)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__peers)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__pending)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__ping)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__ring)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__tethering)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__native__subcmd__unpair)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__notifications)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__notify)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__screensaver)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__send__subcmd__file)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        handoverctl__subcmd__send__subcmd__url)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
    esac
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -F _handoverctl -o nosort -o bashdefault -o default handoverctl
else
    complete -F _handoverctl -o bashdefault -o default handoverctl
fi


_handoverctl_live_devices() {
    COMPREPLY=( $(compgen -W "$(handoverctl devices 2>/dev/null | awk 'NR > 1 && NF { print $1 }')" -- "${cur}") )
}
_handoverctl_generated_device_completion() {
    local prev="${COMP_WORDS[COMP_CWORD-1]}"
    local cur="${COMP_WORDS[COMP_CWORD]}"
    case "${prev}" in
        send-url|send-file|notify|clipboard|calls|cancel-share|ping|ring|lock|keep-awake|tethering|filesystem-list|call|sync)
            _handoverctl_live_devices
            return 0
            ;;
    esac
    _handoverctl_generated "$@"
}
eval "$(declare -f _handoverctl | sed '1s/^_handoverctl /_handoverctl_generated /')"
_handoverctl() { _handoverctl_generated_device_completion "$@"; }
