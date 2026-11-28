# Catàleg de l'intèrfície de Parla (català).

## Accions (títols a la paleta d'ordres i a la documentació del mapa de tecles)
action-app-quit = Surt de Parla
action-app-open-settings = Obre els paràmetres
action-app-open-keymap = Obre el mapa de tecles
action-app-command-palette = Paleta d'ordres
action-app-reload-config = Torna a carregar la configuració
action-app-check-for-updates = Cerca actualitzacions
action-app-about = Quant a Parla
action-theme-toggle-mode = Canvia entre mode clar i fosc
action-store-rebuild-cache = Reconstrueix la memòria cau
action-mcp-rebuild-audit-log = Reconstrueix el registre d'auditoria
action-auth-sign-in-again = Torna a iniciar la sessió
action-auth-sign-out = Tanca la sessió
action-workspace-remove-workspace = Treu aquest espai de treball de Parla
action-app-delete-all-data = Esborra totes les dades de Parla

## Errors del proveïdor que no porten text propi
error-session-expired = la sessió ha caducat
error-not-allowed = no permès: { $detail }
error-rate-limited = massa peticions
error-network = error de xarxa: { $detail }
error-detail = { $detail }
error-unsupported = no disponible aquí
error-too-large = massa gran
error-removal-running = l'espai de treball continua actiu

## Finestra
shell-no-accounts = Encara no hi ha comptes. Inicia la sessió per començar.
shell-connecting = Connectant...
shell-still-connecting = Encara connectant...
shell-select-conversation = Tria una conversa
shell-retry = Torna-ho a provar
shell-start-failed = No s'ha pogut iniciar l'espai de treball.
shell-load-failed = No s'ha pogut carregar l'espai de treball: { $reason }
shell-unknown-provider = Aquest compte pertany a un proveïdor desconegut ({ $provider }).
shell-bootstrap-timeout = el servidor no ha respost a temps
shell-cancel = Cancel·la
shell-remove-title = Vols treure { $name } de Parla?
shell-remove-detail = Es tancarà la sessió i s'esborraran les dades d'aquest espai de treball en aquest dispositiu: els missatges a la memòria cau, els esborranys i els missatges en cua sense enviar es perdran. No s'esborra res al servidor.
shell-remove-confirm = Treu l'espai de treball
shell-set-status = Estableix un estat...
shell-set-away = Posa't com a absent
shell-set-active = Posa't com a actiu
shell-delete-all-title = Vols esborrar totes les dades de Parla?
shell-delete-all-detail = Es tancarà la sessió de tots els espais de treball i s'esborrarà tot el que Parla desa en aquest dispositiu: memòries cau, esborranys, missatges en cua, sessions desades i la base de dades local. Es conserven els teus paràmetres, el mapa de tecles i els temes.
shell-delete-all-confirm = Esborra totes les dades
shell-delete-message-title = Vols eliminar el missatge?
shell-delete-message-confirm = Elimina
shell-removal-failed = No s'ha pogut acabar d'esborrar les dades: { $reason }
menu-view-details = Mostra els detalls
menu-star = Destaca
menu-move-to = Mou a { $section }
menu-move-to-channels = Mou a Canals
menu-move-to-direct-messages = Mou a Missatges directes
menu-sidebar-settings = Paràmetres de la barra lateral…
menu-organise-sidebar = Organitza la barra lateral
menu-unstar = Treu dels destacats
menu-mark-read = Marca com a llegit
menu-copy-link = Copia l'enllaç
menu-edit-channel = Edita el canal…
menu-add-people = Afegeix persones…
menu-leave-channel = Surt del canal
menu-archive-channel = Arxiva el canal
shell-pop-out = Obre en una finestra nova
shell-notify-settings = Tots els ajustos de notificacions…
shell-show-details = Mostra els detalls del canal
shell-hide-details = Amaga els detalls del canal
shell-pop-out-title = { $conversation } · { $workspace }

## Avisos
expiry-message = La sessió ({ $workspace }) ha caducat.
expiry-detail = Encara pots llegir els missatges de la memòria cau.
expiry-sign-in-again = Torna a iniciar la sessió
offline-message = Estàs sense connexió.
offline-detail = Els missatges que enviïs queden en cua i surten en reconnectar. Parla continua intentant-ho.
offline-retry-now = Reintenta ara
store-banner-unreadable = No es pot obrir la memòria cau local d'aquest espai de treball. Els missatges tornaran a carregar-se des de { $provider }. Els missatges en cua i els esborranys desats en aquest dispositiu no es poden recuperar.
store-banner-rebuild = Reconstrueix la memòria cau
store-full-message = Disc gairebé ple
store-full-detail = El disc que conté el magatzem local de Parla té menys d'1 GiB lliure.
store-full-manage = Gestiona l'emmagatzematge
store-notice-unencrypted = El xifratge de disc complet està desactivat: les memòries cau d'imatges es desen sense xifrar en aquest dispositiu.
mcp-notice-socket-unsafe = MCP: la carpeta { $path } no és privada per al teu usuari, així que MCP no escolta. Fes-la teva amb mode 0700 i reinicia Parla.
mcp-notice-audit = MCP: no es pot obrir el registre d'auditoria. Executa "Reconstrueix el registre d'auditoria" per començar-ne un de nou.
warning-keymap = mapa de tecles: { $detail }
warning-keymap-bad-keystrokes = mapa de tecles: tecles no vàlides { $keystrokes }

## Fitxers
file-error-no-download = Aquest fitxer no es pot baixar
file-error-download-failed = La baixada ha fallat: { $reason }
file-error-needs-connection = Les pujades necessiten connexió: connecta't i torna-ho a provar (les pujades no es posen en cua)
file-error-refused = { $reasons }

## Visor de fitxers
media-viewer-meta = { $sender } a { $place } · { $time } · { $size }
media-viewer-meta-direct = { $sender } · { $time } · { $size }
media-viewer-dimensions = { $width } × { $height }
media-viewer-percent = { $value } %
media-viewer-zoom-in = Amplia
media-viewer-zoom-out = Redueix
media-viewer-actual-size = Mida real
media-viewer-fit = Ajusta a la finestra
media-viewer-copy = Copia la imatge
media-viewer-copied = Imatge copiada
media-viewer-download = Baixa
media-viewer-open-external = Obre a l'app predeterminada
media-viewer-close = Tanca el visor
media-viewer-today = Avui { $time }
media-viewer-yesterday = Ahir { $time }
media-viewer-open-in-channel = Obre al canal
media-viewer-prev = Fitxer anterior
media-viewer-next = Fitxer següent
media-viewer-position = { $index } de { $count }
media-viewer-position-in = { $index } de { $count } a { $place }
media-viewer-loading = S'està carregant la imatge
media-viewer-failed = Aquesta imatge no es pot mostrar aquí. Obre-la a l'app predeterminada.
media-viewer-pdf = PDF · { $size }
media-viewer-pdf-note = Parla no dibuixa les pàgines d'un PDF. Obre el fitxer a l'app predeterminada per llegir-lo.

## Quant a
about-version = Parla { $version }
about-licence = Llicència: MPL-2.0
about-privacy = Privadesa
about-terms = Condicions
about-licences = Llicències de tercers
about-source = Codi font
about-security = Informar d'un problema de seguretat
about-name = Parla
about-tagline = Un client de xat d'escriptori petit i ràpid.
about-version-label = Versió
about-licence-label = Llicència
about-copy-version = Copia la informació de versió
about-copied = Copiat
about-made-by = Fet pels col·laboradors de Parla. Documentació CC-BY-4.0.

## Notificacions
notification-action-reply = Respon
notification-action-mark-read = Marca com a llegit
notification-title-channel = { $author } a #{ $channel }
notification-body-files =
    { $count ->
        [one] { $count } fitxer
       *[other] { $count } fitxers
    }
notification-reminder-title = Recordatori
reminder-about-message = Fes el seguiment d'aquest missatge
session-expired-title = Torna a iniciar la sessió a { $workspace }
session-expired-body = La teva sessió ha acabat. Fes clic per iniciar la sessió.
notifications-permission-denied = Les notificacions de Parla estan desactivades als Ajustos del Sistema. Activa-les a Notificacions > Parla.

## Missatges programats
schedule-late-title =
    { $count ->
        [one] Vols enviar el missatge programat que ha vençut?
       *[other] Vols enviar els { $count } missatges programats que han vençut?
    }
schedule-late-detail =
    { $count ->
        [one] Aquest missatge es va programar per a una hora en què Parla no estava obert.
       *[other] Aquests missatges es van programar per a una hora en què Parla no estava obert.
    }
schedule-late-keep = Mantén programat per a més tard
schedule-late-send = Envia ara
schedule-late-line = { $label }: { $text }
action-workspace-pause-notifications = Posa en pausa les notificacions
action-workspace-set-status = Estableix un estat
action-workspace-pop-out-conversation = Obre la conversa en una finestra nova
action-workspace-pop-out-thread = Obre el fil en una finestra nova
message-edit-failed = No s'ha pogut editar el missatge. S'ha deixat tal com estava.
message-delete-failed = No s'ha pogut eliminar el missatge. Encara hi és.
huddle-open-in = Obre aquest huddle a { $provider }
call-fallback-browser = No s'ha pogut obrir la finestra de la trucada; s'ha obert al navegador
call-fallback-app = No s'ha pogut obrir la finestra de la trucada; s'ha obert a l'app del proveïdor
call-native-fallback-browser = No s'ha pogut unir amb àudio natiu; s'ha obert al navegador
call-native-fallback-app = No s'ha pogut unir amb àudio natiu; s'ha obert a l'app del proveïdor
call-native-no-mic = El micròfon no està disponible; pots escoltar però no parlar
call-native-no-speaker = L'altaveu no està disponible; revisa la sortida d'àudio
call-person-unknown = Algú
call-native-failed = La trucada ha acabat inesperadament
call-helper-crashed = La finestra de la trucada s'ha tancat inesperadament
call-switch-title = Vols sortir del huddle actual i unir-te a aquest?
call-switch-leave = Surt i uneix-me
call-switch-stay = Queda't
call-window-title = Huddle a { $name }
call-window-title-empty = Huddle
action-call-toggle-mute = Silencia o activa el micròfon
action-call-leave = Surt del huddle
action-call-show-window = Mostra la finestra de la trucada

## Trucades
ring-title = { $caller } et truca a un huddle
ring-body = Huddle entrant
ring-join = Uneix-te
ring-decline = Rebutja
ring-missed-title = Huddle perdut de { $caller }

## Icona de la barra de menús
tray-open = Obre Parla
tray-quit = Surt de Parla
tray-pause = Posa en pausa les notificacions
tray-resume = Reprèn les notificacions
tray-header =
    { $mentions ->
        [one] { $unread } sense llegir, { $mentions } menció
       *[other] { $unread } sense llegir, { $mentions } mencions
    }
tray-set-status = Canvia l'estat…
tray-settings = Paràmetres…
tray-workspace-mentions = { $name } ({ $count })
tray-workspace-expired = { $name } (torna a iniciar la sessió)
tray-tooltip = Parla
tray-tooltip-mentions =
    { $count ->
        [one] Parla, { $count } menció
       *[other] Parla, { $count } mencions
    }
action-mcp-open-audit-log = Obre el registre d'auditoria de MCP

## MCP status
mcp-status-off = MCP: desactivat
mcp-status-on =
    { $count ->
        [0] MCP: activat, sense clients
        [one] MCP: activat, { $count } client
       *[other] MCP: activat, { $count } clients
    }

## Informes d'errors
crash-notice = Parla s'ha tancat inesperadament. Informe desat localment.
crash-show = Mostra
crash-delete = Elimina
action-app-show-crash-reports = Mostra els informes d'errors

## Actualitzacions
update-available = Parla { $version } està disponible
update-download = Baixa
update-skip = Omet aquesta versió
update-up-to-date = Parla està actualitzat
update-check-failed = No s'ha pogut comprovar si hi ha actualitzacions
update-install = Actualitza
update-retry = Torna-ho a provar
update-downloading = Baixant Parla { $version }… { $percent }%
update-installing = Instal·lant Parla { $version }…
update-ready = Parla { $version } està a punt. Reinicia per actualitzar.
update-restart = Reinicia per actualitzar
update-dismiss = Tanca aquest avís
update-download-failed = No s'ha pogut baixar Parla { $version }
update-signature-invalid = Parla { $version } no ha superat la comprovació de la signatura i no s'ha instal·lat
update-install-failed = No s'ha pogut instal·lar Parla { $version }

titlebar-back = Enrere
titlebar-forward = Endavant
titlebar-search = Cerca a { $workspace }
titlebar-search-plain = Cerca

## Finestra de paràmetres
settings-title = Paràmetres
settings-open-file = Obre settings.json
settings-page-general = General
settings-page-appearance = Aparença
settings-page-notifications = Notificacions
settings-page-keymap = Tecles
settings-page-workspaces = Espais de treball
settings-page-privacy = Privadesa i emmagatzematge
settings-page-calls = Trucades
settings-page-ai = IA
settings-page-plugins = Connectors
settings-page-mcp = MCP
settings-page-advanced = Avançat
settings-footer-version = Parla { $version }
settings-footer-file = Fitxer de paràmetres: { $path }
settings-placeholder-title = Encara no és en aquesta finestra
settings-placeholder-body = Aquests paràmetres encara no són a la finestra de paràmetres. Edita'ls a settings.json; Parla aplica a l'instant els canvis desats.
settings-general-title = General
settings-general-subtitle = Idioma, inici i com viu Parla a l'escriptori.
settings-language-title = Idioma
settings-language-desc = L'aplicació segueix l'idioma del sistema tret que en triïs un altre. Les dates i els números també el segueixen.
settings-language-label = Idioma de la interfície
settings-language-system = Predeterminat del sistema
settings-startup-title = Inici
settings-startup-desc = Parla no fa servir CPU en repòs, així que és segur deixar-lo obert.
settings-login-label = Obre Parla en iniciar la sessió
settings-login-help = S'inicia ocult a la barra de menús
settings-restore-label = Restaura les finestres de l'última sessió
settings-restore-help = S'aplica la propera vegada que s'iniciï Parla.
settings-dock-title = Dock i barra de menús
settings-dock-desc = On es mostra la icona de Parla a macOS.
settings-dock-always = Dock i barra de menús
settings-dock-always-desc = Sempre visible en tots dos.
settings-dock-window = Dock amb finestra
settings-dock-window-desc = Icona al Dock només mentre hi ha una finestra oberta.
settings-dock-never = Només barra de menús
settings-dock-never-desc = Sense icona al Dock.
settings-messages-title = Missatges
settings-messages-desc = Com es mostren l'hora i els fitxers multimèdia a les converses.
settings-clock-label = Rellotge
settings-clock-24 = 24 hores · 16:42
settings-clock-12 = 12 hores · 4:42 PM
settings-images-label = Imatges en línia
settings-images-always = Sempre
settings-images-click = En fer clic
settings-images-never = Mai
settings-updates-title = Actualitzacions
settings-updates-desc = Versions signades del projecte, comprovades un cop al dia.
settings-updates-auto = Cerca actualitzacions automàticament
settings-updates-status = Tens la versió { $version }.
settings-updates-checked = Comprovat a les { $time }.
settings-updates-check-now = Cerca ara
settings-error-write = No s'ha pogut desar settings.json: { $reason }
settings-error-not-object = settings.json no és un objecte JSON, així que no s'ha desat res. Corregeix el fitxer o esborra'l.
settings-appearance-title = Aparença
settings-appearance-subtitle = Fes Parla teu. Els canvis s'apliquen a l'instant i es desen a settings.json.
settings-theme-title = Tema
settings-theme-desc = Segueix el sistema o fixa'n un. Cada mode té el seu propi tema.
settings-theme-system = Sistema
settings-theme-light = Clar
settings-theme-dark = Fosc
settings-accent-title = Accent
settings-accent-desc = Botons, focus i selecció. Tots els ajustos compleixen AA en clar i fosc.
settings-accent-brand = Marca
settings-accent-ember = Brasa
settings-accent-amber = Ambre
settings-accent-coral = Coral
settings-accent-ink = Tinta
settings-accent-high-contrast = El contrast alt està activat, així que l'accent no canvia.
settings-composer-title = Editor de missatges
settings-composer-desc = Com envia el quadre de missatge.
settings-send-key-row = Tecla d'enviament
settings-send-key-row-desc = Amb Intro, Maj-Intro fa un salt de línia; amb ⌘⏎, el fa Intro.
settings-send-key-enter = Intro envia
settings-send-key-cmd-enter-mac = ⌘⏎ envia
settings-send-key-cmd-enter = Ctrl+⏎ envia
settings-density-title = Densitat
settings-density-desc = Alçada de les files a la barra lateral i espaiat dels missatges.
settings-density-compact = Compacta
settings-density-cozy = Còmoda
settings-sidebar-title = Barra lateral
settings-sidebar-desc = Què mostra la llista de converses.
settings-sidebar-unreads = Mostra la secció Sense llegir
settings-sidebar-sort = Ordena les converses
settings-sidebar-sort-alpha = De la A a la Z
settings-sidebar-sort-recent = Recents
settings-appearance-footer = Cada elecció s'escriu a settings.json. Els temes són fitxers JSON: comparteix el teu o deixa el d'una altra persona a la carpeta de temes.
settings-notifications-title = Notificacions
settings-notifications-subtitle = Decideix què t'arriba, com sona i quan Parla guarda silenci.
settings-desktop-title = Notificacions d'escriptori
settings-desktop-desc = Notificacions natives del teu sistema.
settings-desktop-all = Tots els missatges
settings-desktop-all-desc = Cada missatge nou
settings-desktop-mentions = Mencions i missatges directes
settings-desktop-mentions-desc = Recomanat
settings-desktop-none = Cap
settings-desktop-none-desc = Només globus d'avís
settings-preview-label = Mostra la vista prèvia dels missatges
settings-preview-help = Desactivat: la notificació només indica qui ha escrit i on.
settings-keywords-title = Paraules clau
settings-keywords-desc = Tracta aquestes paraules com una menció.
settings-keywords-add = Afegeix una paraula clau...
settings-sound-title = So
settings-sound-desc = El so propi de Parla. El volum segueix el sistema.
settings-sound-label = Reprodueix un so a les notificacions
settings-quiet-title = Hores de silenci
settings-quiet-desc = Atura les notificacions segons un horari. Les mencions continuen marcant.
settings-quiet-from = Des de
settings-quiet-to = fins a
settings-quiet-mon = Dl
settings-quiet-mon-name = dilluns
settings-quiet-tue = Dt
settings-quiet-tue-name = dimarts
settings-quiet-wed = Dc
settings-quiet-wed-name = dimecres
settings-quiet-thu = Dj
settings-quiet-thu-name = dijous
settings-quiet-fri = Dv
settings-quiet-fri-name = divendres
settings-quiet-sat = Ds
settings-quiet-sat-name = dissabte
settings-quiet-sun = Dg
settings-quiet-sun-name = diumenge
settings-time-placeholder = HH:MM
settings-time-invalid = Usa una hora de 24 hores, com 08:30
settings-huddles-title = Huddles
settings-huddles-desc = Huddles entrants de missatges directes.
settings-huddles-ring = Sona als huddles directes entrants
settings-unit-mb = MB
settings-unit-days = dies
settings-unit-messages = miss.
settings-unit-minutes = min
settings-applies-on-restart = S'aplica la propera vegada que s'iniciï Parla.
settings-mcp-title = MCP
settings-mcp-subtitle = Deixa que les eines d'IA d'aquest ordinador llegeixin i, si ho permets, actuïn als teus espais de treball. Desactivat per defecte.
settings-mcp-server-title = Servidor MCP local
settings-mcp-server-desc = Els clients només arriben a Parla a través d'ell, en aquest ordinador.
settings-mcp-enabled = Executa el servidor MCP
settings-mcp-connect-title = Connecta un client
settings-mcp-connect-desc = Com parlen els clients amb Parla.
settings-mcp-config = Configuració del client
settings-mcp-config-help = Copia una configuració que llança aquesta còpia de Parla.
settings-mcp-copy-config = Copia la configuració del client
settings-mcp-copied = Copiat
settings-mcp-workspaces-title = Espais de treball
settings-mcp-workspaces-desc = Els clients només veuen els espais de treball que comparteixes.
settings-mcp-no-workspaces = Encara no has iniciat sessió a cap espai de treball.
settings-mcp-ws-enabled = Comparteix amb clients MCP
settings-mcp-ws-writes = Permet accions d'escriptura
settings-mcp-ws-confirm = Pregunta abans de cada escriptura
settings-mcp-tools-title = Eines · { $count }
settings-mcp-tools-desc = El que pot cridar un client. Les d'escriptura necessiten Permet accions d'escriptura.
settings-mcp-tool-read = LECTURA
settings-mcp-tool-write = ESCRIPTURA
settings-mcp-audit-title = Registre d'auditoria
settings-mcp-audit-desc = Cada crida a una eina, desada en aquest ordinador.
settings-mcp-audit = Activitat d'agents
settings-mcp-audit-help = Crides, decisions i resultats.
settings-mcp-audit-open = Obre el registre
settings-calls-title = Trucades i huddles
settings-calls-subtitle = El timbre i com t'hi uneixes.
settings-calls-ringing-title = Timbre
settings-calls-ringing-desc = Quan algú inicia un huddle amb tu en un missatge directe.
settings-calls-ring = Sona als huddles directes entrants
settings-calls-ringtone = To
settings-calls-ringtone-chime = Campaneta de Parla
settings-calls-ringtone-none = Silenci (només l'avís)
settings-calls-join-title = Uneix-t'hi amb
settings-calls-join-desc = Les trucades d'altres serveis sempre s'obren a la seva pròpia app.
settings-calls-join-native = Àudio de Parla
settings-calls-join-window = Finestra de Parla
settings-calls-join-browser = Navegador
settings-calls-join-provider = App del proveïdor
settings-calls-join-help = L'àudio de Parla s'uneix sense finestra; si no pot connectar, Parla prova la finestra, després el navegador i després l'app del proveïdor.
settings-calls-mute = Silencia el micròfon en unir-t'hi
settings-calls-devices-title = Dispositius d'àudio
settings-calls-devices-desc = Per a l'àudio de Parla. Els canvis s'apliquen a l'instant a la trucada en curs.
settings-calls-input = Micròfon
settings-calls-output = Altaveu
settings-calls-device-default = Predeterminat del sistema
settings-accounts-title = Espais de treball i privadesa
settings-accounts-subtitle = Les teves sessions i el que Parla desa en aquest ordinador.
settings-accounts-list-title = Espais de treball
settings-accounts-list-desc = Cada espai de treball té la seva pròpia sessió.
settings-accounts-none = No has iniciat sessió a cap espai de treball.
settings-accounts-connected = Connectat
settings-accounts-expired = Sessió caducada
settings-accounts-stopped = Sense connexió
settings-accounts-sign-in = Torna a iniciar sessió
settings-accounts-sign-out = Tanca la sessió
settings-accounts-sign-out-title = Vols tancar la sessió a { $name }?
settings-accounts-sign-out-detail = Parla esborra l'inici de sessió desat d'aquest espai de treball. Torna a iniciar sessió per fer-lo servir.
settings-store-title = Magatzem local
settings-store-desc = Els missatges es desen a la memòria cau en una base de dades xifrada per espai de treball.
settings-store-cache = Límit de la memòria cau
settings-store-retention = Desa els missatges durant
settings-store-clear-label = Buida la memòria cau
settings-store-clear-help = Esborra els missatges de la memòria cau; cada espai de treball torna a carregar.
settings-privacy-title = Privadesa
settings-privacy-desc = Com protegeix Parla el que desa en aquest dispositiu.
settings-lock-idle = Bloqueja el magatzem en inactivitat
settings-lock-idle-help = Després d'aquest temps sense activitat, Parla tanca el magatzem xifrat, en treu les claus de la memòria i cobreix les finestres fins que el desbloquegis. S'aplica de seguida.
settings-lock-off = No
settings-lock-minutes = { $n } min
settings-lock-hour = 1 hora
settings-crash = Informes d'errors
settings-crash-help = Un informe sense dades sensibles es queda en aquest dispositiu i no s'envia mai. Pregunta ofereix mostrar-lo o eliminar-lo en el següent inici.
settings-crash-off = No
settings-crash-ask = Pregunta sempre
settings-crash-local = Desa en local
settings-store-clear = Buida la memòria cau
settings-store-clear-title = Vols buidar la memòria cau de missatges?
settings-store-clear-detail = Cada espai de treball esborra els missatges de la memòria cau i els esborranys i torna a carregar des del servidor.
settings-danger-title = Zona de perill
settings-danger-desc = No es pot desfer.
settings-danger-delete = Esborra les dades locals i tanca totes les sessions
settings-danger-delete-help = Treu el magatzem xifrat i els inicis de sessió desats. La configuració es manté.
settings-danger-delete-button = Esborra…
settings-danger-reset-label = Restableix tota la configuració
settings-danger-reset-help = Torna als valors per defecte. Els fitxers de temes i de dreceres es queden al disc.
settings-danger-reset = Restableix la configuració
settings-danger-reset-title = Vols restablir tota la configuració?
settings-danger-reset-detail = settings.json torna als valors per defecte comentats. Els teus temes i keymap.json es conserven.
settings-advanced-title = Avançat
settings-advanced-subtitle = Memòria, registres i recuperació. Els valors per defecte mantenen Parla lleuger; canvia'ls només si ho necessites.
settings-memory-title = Memòria
settings-memory-desc = Límits superiors. Parla no creix mai més enllà.
settings-advanced-image-cache = Memòria cau d'imatges
settings-advanced-image-cache-help = S'aplica la propera vegada que s'iniciï Parla.
settings-advanced-message-window = Missatges en memòria per conversa
settings-advanced-lean = Mode lleuger després d'amagar la finestra durant
settings-step-cache-dec = Redueix el límit de la memòria cau
settings-step-cache-inc = Augmenta el límit de la memòria cau
settings-step-retention-dec = Desa els missatges menys temps
settings-step-retention-inc = Desa els missatges més temps
settings-step-image-cache-dec = Redueix la memòria cau d'imatges
settings-step-image-cache-inc = Augmenta la memòria cau d'imatges
settings-step-message-window-dec = Desa menys missatges en memòria
settings-step-message-window-inc = Desa més missatges en memòria
settings-step-lean-dec = Passa abans al mode lleuger
settings-step-lean-inc = Passa més tard al mode lleuger
action-app-open-settings-file = Obre settings.json

## Menu bar
action-app-hide = Amaga Parla
action-app-hide-others = Amaga les altres
action-app-show-all = Mostra-ho tot
action-app-minimize = Minimitza
action-app-zoom = Amplia
action-app-close-window = Tanca la finestra
action-app-bring-all-to-front = Porta-ho tot al davant
action-app-open-docs = Documentació de Parla
action-app-open-keyboard-shortcuts = Dreceres de teclat
action-app-report-issue = Informa d'un problema
menu-settings = Paràmetres…
menu-edit = Edita
menu-view = Visualitza
menu-window = Finestra
menu-help = Ajuda
menu-undo = Desfés
menu-redo = Refés
menu-cut = Retalla
menu-copy = Copia
menu-paste = Enganxa
menu-select-all = Selecciona-ho tot
shell-switch-to = Canvia a { $workspace }
shell-dm-unsupported = Encara no es pot escriure a { $person } des d'aquí.
shell-dm-failed = No s'ha pogut obrir una conversa amb { $person }: { $reason }
shell-admin-failed = No ha funcionat: { $reason }
shell-join-failed = No t'has pogut unir al canal
shell-invite-title = Afegeix persones a #{ $channel }
shell-invite-person-failed = { $person }: { $reason }
shell-leave-title = Vols sortir de #{ $channel }?
shell-leave-body = Hi pots tornar a entrar des d'Explora els canals.
shell-leave-body-private = És privat: necessitaràs una invitació per tornar-hi.
shell-leave-ok = Surt
shell-archive-title = Vols arxivar #{ $channel }?
shell-archive-body = Ningú hi podrà publicar. Els missatges es podran continuar llegint i cercant.
shell-archive-ok = Arxiva

## Keyboard shortcuts (the sheet and the Settings Keymap page)
shortcuts-area-navigation = Navegació
shortcuts-area-messages = Missatges
shortcuts-area-composer = Redactor
shortcuts-area-calls = Trucades
shortcuts-area-app = App i finestres
shortcuts-workspace-range = Canvia a l'espai de treball 1–9
shortcuts-area-text = Edició de text
settings-keymap-subtitle = Cada acció té una drecera que pots canviar. Les teves dreceres són a keymap.json i s'apliquen sobre les predeterminades.
settings-keymap-open-file = Obre keymap.json
settings-keymap-search = Cerca per acció o tecles
settings-keymap-count = { $shown } de { $total } accions
settings-keymap-filter-all = Totes
settings-keymap-filter-changed = Canviades
settings-keymap-filter-conflicts = Conflictes · { $count }
settings-keymap-needs-modifier = Afegeix un modificador com Cmd, Ctrl o Alt: aquí una tecla sola s'activaria mentre escrius.
settings-keymap-shadow = També executa { $action }. Mentre { $context } té el focus, guanya la seva drecera.
settings-keymap-col-action = Acció
settings-keymap-col-shortcut = Drecera
settings-keymap-col-context = Context
settings-keymap-col-source = Origen
settings-keymap-context-any = A tot arreu
settings-keymap-source-default = Predeterminada
settings-keymap-source-user = Tu
settings-keymap-unbound = Sense assignar
settings-keymap-edit-label = Canvia la drecera de { $action }
settings-keymap-press-keys = Prem les tecles…
settings-keymap-save = Desa
settings-keymap-cancel = Cancel·la
settings-keymap-reset = Restableix
settings-keymap-conflict = També executa { $action } aquí. Si deses, la substitueix.
settings-keymap-footer = Retorn desa i Esc cancel·la; Tab no es pot enregistrar. Una segona tecla crea una seqüència com Cmd-K Cmd-S. Els contextos s'imbriquen: les dreceres del redactor guanyen a les globals mentre escrius.
settings-keymap-error-not-array = keymap.json no és un array JSON, així que no s'ha desat res. Corregeix el fitxer o esborra'l.

settings-logs-title = Registres i diagnòstic
settings-logs-desc = Els tokens, les cookies i el text dels missatges no entren al registre ni a l'exportació.
settings-log-level = Nivell de registre
settings-log-level-help = Quant escriu Parla al seu registre. S'aplica a l'instant.
settings-log-error = Error
settings-log-info = Info
settings-log-debug = Depuració
settings-open-logs = Obre la carpeta de registres
settings-open-logs-label = Registres
settings-open-logs-help = Mostra parla.log al teu gestor de fitxers.
settings-export-diagnostics = Exporta…
settings-export-label = Diagnòstic redactat per a un informe d'error
settings-export-help = Desa un fitxer de text amb la versió, els ajustos, les últimes línies del registre, els comptadors i l'últim informe d'errors, i el mostra. Llegeix-lo abans de compartir-lo.
settings-error-export = No s'ha pogut exportar el diagnòstic: { $reason }
lock-title = Parla està bloquejat
lock-detail = Parla ha tancat el magatzem xifrat després d'una estona sense ús. Desbloqueja'l per obrir-lo de nou amb la clau del clauer.
lock-unlock = Desbloqueja
lock-error = No s'ha pogut llegir la clau del magatzem xifrat del clauer. Torna a prémer Desbloqueja.

code-viewer-meta = { $sender } a { $place } · { $time }
code-viewer-meta-direct = { $sender } · { $time }
code-viewer-jump = Ves al missatge
code-viewer-wrap = Ajusta
code-viewer-line-numbers = Números de línia
code-viewer-copy = Copia
code-viewer-copied = Copiat
code-viewer-download = Baixa
code-viewer-find-placeholder = Cerca al fitxer
code-viewer-find-count = { $index } de { $count }
code-viewer-find-count-capped = { $index } de { $count }+
code-viewer-find-none = Cap resultat
code-viewer-find-count-label = Coincidència { $index } de { $count }
code-viewer-find-count-label-capped = Coincidència { $index } de més de { $count }
code-viewer-region =
    { $count ->
        [one] Codi, 1 línia
       *[other] Codi, { $count } línies
    }
code-viewer-find-prev = Coincidència anterior
code-viewer-find-next = Coincidència següent
code-viewer-find-close = Tanca la cerca
code-viewer-loading = Carregant el fitxer sencer
code-viewer-failed = No s'ha pogut carregar el fitxer sencer; es mostra la previsualització.
code-viewer-failed-empty = No s'ha pogut carregar aquest fitxer. Baixa'l.
code-viewer-too-large = Aquest fitxer és massa gran per mostrar-lo aquí. Baixa'l.
code-viewer-not-text = Aquest fitxer no és text. Baixa'l.
code-viewer-lines =
    { $count ->
        [one] 1 línia
       *[other] { $count } línies
    }
