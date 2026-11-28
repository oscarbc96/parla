# Catálogo del shell de Parla (español).

## Acciones (títulos en la paleta de comandos y en la documentación del mapa de teclas)
action-app-quit = Salir de Parla
action-app-open-settings = Abrir los ajustes
action-app-open-keymap = Abrir el mapa de teclas
action-app-command-palette = Paleta de comandos
action-app-reload-config = Recargar la configuración
action-app-check-for-updates = Buscar actualizaciones
action-app-about = Acerca de Parla
action-theme-toggle-mode = Alternar entre modo claro y oscuro
action-store-rebuild-cache = Reconstruir la caché
action-mcp-rebuild-audit-log = Reconstruir el registro de auditoría
action-auth-sign-in-again = Iniciar sesión de nuevo
action-auth-sign-out = Cerrar sesión
action-workspace-remove-workspace = Quitar este espacio de trabajo de Parla
action-app-delete-all-data = Borrar todos los datos de Parla

## Errores del proveedor que no traen texto propio
error-session-expired = la sesión ha caducado
error-not-allowed = no permitido: { $detail }
error-rate-limited = demasiadas peticiones
error-network = error de red: { $detail }
error-detail = { $detail }
error-unsupported = no disponible aquí
error-too-large = demasiado grande
error-removal-running = el espacio de trabajo sigue activo

## Ventana
shell-no-accounts = Todavía no hay cuentas. Inicia sesión para empezar.
shell-connecting = Conectando...
shell-still-connecting = Todavía conectando...
shell-select-conversation = Elige una conversación
shell-retry = Reintentar
shell-start-failed = No se ha podido iniciar el espacio de trabajo.
shell-load-failed = No se ha podido cargar el espacio de trabajo: { $reason }
shell-unknown-provider = Esta cuenta pertenece a un proveedor desconocido ({ $provider }).
shell-bootstrap-timeout = el servidor no ha respondido a tiempo
shell-cancel = Cancelar
shell-remove-title = ¿Quitar { $name } de Parla?
shell-remove-detail = Se cerrará la sesión y se borrarán los datos de este espacio de trabajo en este dispositivo: los mensajes en caché, los borradores y los mensajes en cola sin enviar se perderán. No se borra nada en el servidor.
shell-remove-confirm = Quitar espacio de trabajo
shell-set-status = Establecer un estado...
shell-set-away = Ponerte como ausente
shell-set-active = Ponerte como activo
shell-delete-all-title = ¿Borrar todos los datos de Parla?
shell-delete-all-detail = Se cerrará la sesión en todos los espacios de trabajo y se borrará todo lo que Parla guarda en este dispositivo: cachés, borradores, mensajes en cola, sesiones guardadas y la base de datos local. Se conservan tus ajustes, el mapa de teclas y los temas.
shell-delete-all-confirm = Borrar todos los datos
shell-delete-message-title = ¿Eliminar mensaje?
shell-delete-message-confirm = Eliminar
shell-removal-failed = No se ha podido terminar de borrar los datos: { $reason }
menu-view-details = Ver detalles
menu-star = Destacar
menu-move-to = Mover a { $section }
menu-move-to-channels = Mover a Canales
menu-move-to-direct-messages = Mover a Mensajes directos
menu-sidebar-settings = Ajustes de la barra lateral…
menu-organise-sidebar = Organizar la barra lateral
menu-unstar = Quitar de destacados
menu-mark-read = Marcar como leído
menu-copy-link = Copiar enlace
menu-edit-channel = Editar canal…
menu-add-people = Añadir personas…
menu-leave-channel = Salir del canal
menu-archive-channel = Archivar canal
shell-pop-out = Abrir en una ventana nueva
shell-notify-settings = Todos los ajustes de notificaciones…
shell-show-details = Mostrar detalles del canal
shell-hide-details = Ocultar detalles del canal
shell-pop-out-title = { $conversation } · { $workspace }

## Avisos
expiry-message = Tu sesión de { $workspace } ha caducado.
expiry-detail = Aún puedes leer los mensajes en caché.
expiry-sign-in-again = Iniciar sesión de nuevo
offline-message = Estás sin conexión.
offline-detail = Los mensajes que envíes quedan en cola y salen al reconectar. Parla sigue intentándolo.
offline-retry-now = Reintentar ahora
store-banner-unreadable = No se puede abrir la caché local de este espacio de trabajo. Los mensajes volverán a cargarse desde { $provider }. Los mensajes en cola y los borradores guardados en este dispositivo no se pueden recuperar.
store-banner-rebuild = Reconstruir la caché
store-full-message = Disco casi lleno
store-full-detail = El disco que contiene el almacén local de Parla tiene menos de 1 GiB libre.
store-full-manage = Gestionar almacenamiento
store-notice-unencrypted = El cifrado de disco completo está desactivado: las cachés de imágenes se guardan sin cifrar en este dispositivo.
mcp-notice-socket-unsafe = MCP: la carpeta { $path } no es privada para tu usuario, así que MCP no escucha. Haz que sea tuya con modo 0700 y reinicia Parla.
mcp-notice-audit = MCP: no se puede abrir el registro de auditoría. Ejecuta "Reconstruir el registro de auditoría" para empezar uno nuevo.
warning-keymap = mapa de teclas: { $detail }
warning-keymap-bad-keystrokes = mapa de teclas: pulsaciones no válidas { $keystrokes }

## Archivos
file-error-no-download = Este archivo no se puede descargar
file-error-download-failed = Ha fallado la descarga: { $reason }
file-error-needs-connection = Las subidas necesitan conexión: conéctate e inténtalo de nuevo (las subidas no se ponen en cola)
file-error-refused = { $reasons }

## Visor de archivos
media-viewer-meta = { $sender } en { $place } · { $time } · { $size }
media-viewer-meta-direct = { $sender } · { $time } · { $size }
media-viewer-dimensions = { $width } × { $height }
media-viewer-percent = { $value } %
media-viewer-zoom-in = Ampliar
media-viewer-zoom-out = Reducir
media-viewer-actual-size = Tamaño real
media-viewer-fit = Ajustar a la ventana
media-viewer-copy = Copiar imagen
media-viewer-copied = Imagen copiada
media-viewer-download = Descargar
media-viewer-open-external = Abrir en la app predeterminada
media-viewer-close = Cerrar el visor
media-viewer-today = Hoy { $time }
media-viewer-yesterday = Ayer { $time }
media-viewer-open-in-channel = Abrir en el canal
media-viewer-prev = Archivo anterior
media-viewer-next = Archivo siguiente
media-viewer-position = { $index } de { $count }
media-viewer-position-in = { $index } de { $count } en { $place }
media-viewer-loading = Cargando la imagen
media-viewer-failed = Esta imagen no se puede mostrar aquí. Ábrela en la app predeterminada.
media-viewer-pdf = PDF · { $size }
media-viewer-pdf-note = Parla no dibuja las páginas de un PDF. Abre el archivo en la app predeterminada para leerlo.

## Acerca de
about-version = Parla { $version }
about-licence = Licencia: MPL-2.0
about-privacy = Privacidad
about-terms = Condiciones
about-licences = Licencias de terceros
about-source = Código fuente
about-security = Informar de un problema de seguridad
about-name = Parla
about-tagline = Un cliente de chat de escritorio pequeño y rápido.
about-version-label = Versión
about-licence-label = Licencia
about-copy-version = Copiar información de versión
about-copied = Copiado
about-made-by = Hecho por los colaboradores de Parla. Documentación CC-BY-4.0.

## Notificaciones
notification-action-reply = Responder
notification-action-mark-read = Marcar como leído
notification-title-channel = { $author } en #{ $channel }
notification-body-files =
    { $count ->
        [one] { $count } archivo
       *[other] { $count } archivos
    }
notification-reminder-title = Recordatorio
reminder-about-message = Hacer seguimiento de este mensaje
session-expired-title = Inicia sesión en { $workspace } de nuevo
session-expired-body = Tu sesión terminó. Haz clic para iniciar sesión.

## Mensajes programados
schedule-late-title =
    { $count ->
        [one] ¿Enviar el mensaje programado que ha vencido?
       *[other] ¿Enviar los { $count } mensajes programados que han vencido?
    }
schedule-late-detail =
    { $count ->
        [one] Este mensaje se programó para una hora en la que Parla no estaba abierto.
       *[other] Estos mensajes se programaron para una hora en la que Parla no estaba abierto.
    }
schedule-late-keep = Mantener programado para más tarde
schedule-late-send = Enviar ahora
schedule-late-line = { $label }: { $text }
action-workspace-pause-notifications = Pausar notificaciones
action-workspace-set-status = Establecer un estado
action-workspace-pop-out-conversation = Abrir conversación en una ventana nueva
action-workspace-pop-out-thread = Abrir hilo en una ventana nueva
message-edit-failed = No se pudo editar el mensaje. Se dejó como estaba.
message-delete-failed = No se pudo eliminar el mensaje. Sigue ahí.
huddle-open-in = Abre este huddle en { $provider }
call-fallback-browser = No se pudo abrir la ventana de la llamada; se abrió en tu navegador
call-fallback-app = No se pudo abrir la ventana de la llamada; se abrió en la app del proveedor
call-native-fallback-browser = No se pudo unir con audio nativo; se abrió en tu navegador
call-native-fallback-app = No se pudo unir con audio nativo; se abrió en la app del proveedor
call-native-no-mic = El micrófono no está disponible; puedes escuchar pero no hablar
call-native-no-speaker = El altavoz no está disponible; revisa la salida de audio
call-person-unknown = Alguien
call-native-failed = La llamada terminó inesperadamente
call-helper-crashed = La ventana de la llamada se cerró inesperadamente
call-switch-title = ¿Salir del huddle actual y unirte a este?
call-switch-leave = Salir y unirme
call-switch-stay = Quedarme
call-window-title = Huddle en { $name }
call-window-title-empty = Huddle
action-call-toggle-mute = Silenciar o activar el micrófono
action-call-leave = Salir del huddle
action-call-show-window = Mostrar la ventana de la llamada

## Llamadas
ring-title = { $caller } te llama a un huddle
ring-body = Huddle entrante
ring-join = Unirse
ring-decline = Rechazar
ring-missed-title = Huddle perdido de { $caller }

## Icono de la barra de menús
tray-open = Abrir Parla
tray-quit = Salir de Parla
tray-pause = Pausar notificaciones
tray-resume = Reanudar notificaciones
tray-header =
    { $mentions ->
        [one] { $unread } sin leer, { $mentions } mención
       *[other] { $unread } sin leer, { $mentions } menciones
    }
tray-set-status = Cambiar estado…
tray-settings = Ajustes…
tray-workspace-mentions = { $name } ({ $count })
tray-workspace-expired = { $name } (inicia sesión de nuevo)
tray-tooltip = Parla
tray-tooltip-mentions =
    { $count ->
        [one] Parla, { $count } mención
       *[other] Parla, { $count } menciones
    }
action-mcp-open-audit-log = Abrir el registro de auditoría de MCP

## MCP status
mcp-status-off = MCP: desactivado
mcp-status-on =
    { $count ->
        [0] MCP: activado, sin clientes
        [one] MCP: activado, { $count } cliente
       *[other] MCP: activado, { $count } clientes
    }

## Informes de fallos
crash-notice = Parla se cerró inesperadamente. Informe guardado localmente.
crash-show = Mostrar
crash-delete = Eliminar
action-app-show-crash-reports = Mostrar informes de fallos

## Actualizaciones
update-available = Parla { $version } está disponible
update-download = Descargar
update-skip = Omitir esta versión
update-up-to-date = Parla está actualizado
update-check-failed = No se pudo comprobar si hay actualizaciones
update-install = Actualizar
update-retry = Reintentar
update-downloading = Descargando Parla { $version }… { $percent }%
update-installing = Instalando Parla { $version }…
update-ready = Parla { $version } está listo. Reinicia para actualizar.
update-restart = Reiniciar para actualizar
update-dismiss = Cerrar este aviso
update-download-failed = No se pudo descargar Parla { $version }
update-signature-invalid = Parla { $version } no superó la comprobación de la firma y no se instaló
update-install-failed = No se pudo instalar Parla { $version }

titlebar-back = Atrás
titlebar-forward = Adelante
titlebar-search = Buscar en { $workspace }
titlebar-search-plain = Buscar
titlebar-help = Ayuda

## Ventana de ajustes
settings-title = Ajustes
settings-open-file = Abrir settings.json
settings-page-general = General
settings-page-appearance = Apariencia
settings-page-notifications = Notificaciones
settings-page-keymap = Teclas
settings-page-workspaces = Espacios de trabajo
settings-page-privacy = Privacidad y almacenamiento
settings-page-calls = Llamadas
settings-page-ai = IA
settings-page-plugins = Complementos
settings-page-mcp = MCP
settings-page-advanced = Avanzado
settings-footer-version = Parla { $version }
settings-footer-file = Archivo de ajustes: { $path }
settings-placeholder-title = Aún no está en esta ventana
settings-placeholder-body = Estos ajustes aún no están en la ventana de ajustes. Edítalos en settings.json; Parla aplica al instante los cambios guardados.
settings-general-title = General
settings-general-subtitle = Idioma, inicio y cómo vive Parla en tu escritorio.
settings-language-title = Idioma
settings-language-desc = La aplicación sigue el idioma del sistema salvo que elijas otro. Las fechas y los números también lo siguen.
settings-language-label = Idioma de la interfaz
settings-language-system = Predeterminado del sistema
settings-startup-title = Inicio
settings-startup-desc = Parla no usa CPU en reposo, así que es seguro dejarlo abierto.
settings-login-label = Abrir Parla al iniciar sesión
settings-login-help = Se inicia oculto en la barra de menús
settings-restore-label = Restaurar las ventanas de la última sesión
settings-restore-help = Se aplica la próxima vez que se inicie Parla.
settings-dock-title = Dock y barra de menús
settings-dock-desc = Dónde se muestra el icono de Parla en macOS.
settings-dock-always = Dock y barra de menús
settings-dock-always-desc = Siempre visible en ambos.
settings-dock-window = Dock con ventana
settings-dock-window-desc = Icono en el Dock solo mientras hay una ventana abierta.
settings-dock-never = Solo barra de menús
settings-dock-never-desc = Sin icono en el Dock.
settings-messages-title = Mensajes
settings-messages-desc = Cómo se muestran la hora y los archivos multimedia en las conversaciones.
settings-clock-label = Reloj
settings-clock-24 = 24 horas · 16:42
settings-clock-12 = 12 horas · 4:42 PM
settings-images-label = Imágenes en línea
settings-images-always = Siempre
settings-images-click = Al hacer clic
settings-images-never = Nunca
settings-updates-title = Actualizaciones
settings-updates-desc = Versiones firmadas del proyecto, comprobadas una vez al día.
settings-updates-auto = Buscar actualizaciones automáticamente
settings-updates-status = Tienes la versión { $version }.
settings-updates-checked = Comprobado a las { $time }.
settings-updates-check-now = Buscar ahora
settings-error-write = No se pudo guardar settings.json: { $reason }
settings-error-not-object = settings.json no es un objeto JSON, así que no se guardó nada. Corrige el archivo o bórralo.
settings-appearance-title = Apariencia
settings-appearance-subtitle = Haz Parla tuyo. Los cambios se aplican al instante y se guardan en settings.json.
settings-theme-title = Tema
settings-theme-desc = Sigue el sistema o fija uno. Cada modo tiene su propio tema.
settings-theme-system = Sistema
settings-theme-light = Claro
settings-theme-dark = Oscuro
settings-accent-title = Acento
settings-accent-desc = Botones, foco y selección. Todos los ajustes cumplen AA en claro y oscuro.
settings-accent-brand = Marca
settings-accent-ember = Brasa
settings-accent-amber = Ámbar
settings-accent-coral = Coral
settings-accent-ink = Tinta
settings-accent-high-contrast = El alto contraste está activado, así que el acento no cambia.
settings-composer-title = Editor de mensajes
settings-composer-desc = Cómo envía el cuadro de mensaje.
settings-send-key-row = Tecla de envío
settings-send-key-row-desc = Con Intro, Mayús-Intro hace un salto de línea; con ⌘⏎, lo hace Intro.
settings-send-key-enter = Intro envía
settings-send-key-cmd-enter-mac = ⌘⏎ envía
settings-send-key-cmd-enter = Ctrl+⏎ envía
settings-density-title = Densidad
settings-density-desc = Altura de las filas en la barra lateral y espaciado de los mensajes.
settings-density-compact = Compacta
settings-density-cozy = Cómoda
settings-sidebar-title = Barra lateral
settings-sidebar-desc = Qué muestra la lista de conversaciones.
settings-sidebar-unreads = Mostrar la sección Sin leer
settings-sidebar-sort = Ordenar conversaciones
settings-sidebar-sort-alpha = De la A a la Z
settings-sidebar-sort-recent = Recientes
settings-appearance-footer = Cada elección se escribe en settings.json. Los temas son archivos JSON: comparte el tuyo o suelta el de otra persona en la carpeta de temas.
settings-notifications-title = Notificaciones
settings-notifications-subtitle = Decide qué te llega, cómo suena y cuándo Parla guarda silencio.
settings-desktop-title = Notificaciones de escritorio
settings-desktop-desc = Notificaciones nativas de tu sistema.
settings-desktop-all = Todos los mensajes
settings-desktop-all-desc = Cada mensaje nuevo
settings-desktop-mentions = Menciones y mensajes directos
settings-desktop-mentions-desc = Recomendado
settings-desktop-none = Ninguna
settings-desktop-none-desc = Solo distintivos
settings-preview-label = Mostrar vista previa de los mensajes
settings-preview-help = Desactivado: la notificación solo indica quién escribió y dónde.
settings-keywords-title = Palabras clave
settings-keywords-desc = Trata estas palabras como una mención.
settings-keywords-add = Añade una palabra clave...
settings-sound-title = Sonido
settings-sound-desc = El sonido propio de Parla. El volumen sigue al sistema.
settings-sound-label = Reproducir un sonido en las notificaciones
settings-quiet-title = Horas de silencio
settings-quiet-desc = Pausa las notificaciones según un horario. Las menciones siguen marcando.
settings-quiet-from = Desde
settings-quiet-to = hasta
settings-quiet-mon = L
settings-quiet-mon-name = lunes
settings-quiet-tue = M
settings-quiet-tue-name = martes
settings-quiet-wed = X
settings-quiet-wed-name = miércoles
settings-quiet-thu = J
settings-quiet-thu-name = jueves
settings-quiet-fri = V
settings-quiet-fri-name = viernes
settings-quiet-sat = S
settings-quiet-sat-name = sábado
settings-quiet-sun = D
settings-quiet-sun-name = domingo
settings-time-placeholder = HH:MM
settings-time-invalid = Usa una hora de 24 horas, como 08:30
settings-huddles-title = Huddles
settings-huddles-desc = Huddles entrantes de mensajes directos.
settings-huddles-ring = Sonar en los huddles directos entrantes
settings-unit-mb = MB
settings-unit-days = días
settings-unit-messages = mens.
settings-unit-minutes = min
settings-applies-on-restart = Se aplica la próxima vez que se inicie Parla.
settings-mcp-title = MCP
settings-mcp-subtitle = Deja que las herramientas de IA de este ordenador lean y, si lo permites, actúen en tus espacios de trabajo. Desactivado por defecto.
settings-mcp-server-title = Servidor MCP local
settings-mcp-server-desc = Los clientes solo llegan a Parla a través de él, en este ordenador.
settings-mcp-enabled = Ejecutar el servidor MCP
settings-mcp-connect-title = Conectar un cliente
settings-mcp-connect-desc = Cómo hablan los clientes con Parla.
settings-mcp-config = Configuración del cliente
settings-mcp-config-help = Copia una configuración que lanza esta copia de Parla.
settings-mcp-copy-config = Copiar configuración del cliente
settings-mcp-copied = Copiado
settings-mcp-workspaces-title = Espacios de trabajo
settings-mcp-workspaces-desc = Los clientes solo ven los espacios de trabajo que compartes.
settings-mcp-no-workspaces = Aún no has iniciado sesión en ningún espacio de trabajo.
settings-mcp-ws-enabled = Compartir con clientes MCP
settings-mcp-ws-writes = Permitir acciones de escritura
settings-mcp-ws-confirm = Preguntar antes de cada escritura
settings-mcp-tools-title = Herramientas · { $count }
settings-mcp-tools-desc = Lo que puede llamar un cliente. Las de escritura necesitan Permitir acciones de escritura.
settings-mcp-tool-read = LECTURA
settings-mcp-tool-write = ESCRITURA
settings-mcp-audit-title = Registro de auditoría
settings-mcp-audit-desc = Cada llamada a una herramienta, guardada en este ordenador.
settings-mcp-audit = Actividad de agentes
settings-mcp-audit-help = Llamadas, decisiones y resultados.
settings-mcp-audit-open = Abrir el registro
settings-calls-title = Llamadas y huddles
settings-calls-subtitle = El timbre y cómo te unes.
settings-calls-ringing-title = Timbre
settings-calls-ringing-desc = Cuando alguien inicia un huddle contigo en un mensaje directo.
settings-calls-ring = Sonar en los huddles directos entrantes
settings-calls-ringtone = Tono
settings-calls-ringtone-chime = Campanilla de Parla
settings-calls-ringtone-none = Silencio (solo el aviso)
settings-calls-join-title = Unirse con
settings-calls-join-desc = Las llamadas de otros servicios siempre se abren en su propia app.
settings-calls-join-native = Audio de Parla
settings-calls-join-window = Ventana de Parla
settings-calls-join-browser = Navegador
settings-calls-join-provider = App del proveedor
settings-calls-join-help = El audio de Parla se une sin ventana; si no puede conectar, Parla prueba la ventana, luego el navegador y luego la app del proveedor.
settings-calls-mute = Silenciar el micrófono al unirte
settings-calls-devices-title = Dispositivos de audio
settings-calls-devices-desc = Para el audio de Parla. Los cambios se aplican al momento en la llamada en curso.
settings-calls-input = Micrófono
settings-calls-output = Altavoz
settings-calls-device-default = Predeterminado del sistema
settings-accounts-title = Espacios de trabajo y privacidad
settings-accounts-subtitle = Tus sesiones y lo que Parla guarda en este ordenador.
settings-accounts-list-title = Espacios de trabajo
settings-accounts-list-desc = Cada espacio de trabajo tiene su propia sesión.
settings-accounts-none = No has iniciado sesión en ningún espacio de trabajo.
settings-accounts-connected = Conectado
settings-accounts-expired = Sesión caducada
settings-accounts-stopped = Sin conexión
settings-accounts-sign-in = Volver a iniciar sesión
settings-accounts-sign-out = Cerrar sesión
settings-accounts-sign-out-title = ¿Cerrar sesión en { $name }?
settings-accounts-sign-out-detail = Parla borra el inicio de sesión guardado de este espacio de trabajo. Vuelve a iniciar sesión para usarlo.
settings-store-title = Almacén local
settings-store-desc = Los mensajes se guardan en caché en una base de datos cifrada por espacio de trabajo.
settings-store-cache = Límite de la caché
settings-store-retention = Guardar los mensajes durante
settings-store-clear-label = Vaciar la caché
settings-store-clear-help = Borra los mensajes en caché; cada espacio de trabajo vuelve a cargar.
settings-privacy-title = Privacidad
settings-privacy-desc = Cómo protege Parla lo que guarda en este dispositivo.
settings-lock-idle = Bloquear el almacén en inactividad
settings-lock-idle-help = Tras este tiempo sin actividad, Parla cierra el almacén cifrado, quita sus claves de la memoria y cubre sus ventanas hasta que lo desbloquees. Se aplica al momento.
settings-lock-off = No
settings-lock-minutes = { $n } min
settings-lock-hour = 1 hora
settings-crash = Informes de fallos
settings-crash-help = Un informe sin datos sensibles se queda en este dispositivo y nunca se envía. Preguntar ofrece mostrarlo o eliminarlo en el siguiente inicio.
settings-crash-off = No
settings-crash-ask = Preguntar siempre
settings-crash-local = Guardar en local
settings-store-clear = Vaciar la caché
settings-store-clear-title = ¿Vaciar la caché de mensajes?
settings-store-clear-detail = Cada espacio de trabajo borra sus mensajes en caché y sus borradores y vuelve a cargar desde el servidor.
settings-danger-title = Zona de peligro
settings-danger-desc = No se puede deshacer.
settings-danger-delete = Borrar los datos locales y cerrar todas las sesiones
settings-danger-delete-help = Quita el almacén cifrado y los inicios de sesión guardados. Los ajustes se quedan.
settings-danger-delete-button = Borrar…
settings-danger-reset-label = Restablecer todos los ajustes
settings-danger-reset-help = Vuelve a los valores por defecto. Los archivos de temas y de atajos se quedan en el disco.
settings-danger-reset = Restablecer ajustes
settings-danger-reset-title = ¿Restablecer todos los ajustes?
settings-danger-reset-detail = settings.json vuelve a los valores por defecto comentados. Tus temas y keymap.json se conservan.
settings-advanced-title = Avanzado
settings-advanced-subtitle = Memoria, registros y recuperación. Los valores por defecto mantienen Parla ligero; cámbialos solo si lo necesitas.
settings-memory-title = Memoria
settings-memory-desc = Límites superiores. Parla nunca crece más allá.
settings-advanced-image-cache = Caché de imágenes
settings-advanced-image-cache-help = Se aplica la próxima vez que se inicie Parla.
settings-advanced-message-window = Mensajes en memoria por conversación
settings-advanced-lean = Modo ligero tras ocultar la ventana durante
settings-step-cache-dec = Reducir el límite de la caché
settings-step-cache-inc = Aumentar el límite de la caché
settings-step-retention-dec = Guardar los mensajes menos tiempo
settings-step-retention-inc = Guardar los mensajes más tiempo
settings-step-image-cache-dec = Reducir la caché de imágenes
settings-step-image-cache-inc = Aumentar la caché de imágenes
settings-step-message-window-dec = Guardar menos mensajes en memoria
settings-step-message-window-inc = Guardar más mensajes en memoria
settings-step-lean-dec = Pasar antes al modo ligero
settings-step-lean-inc = Pasar más tarde al modo ligero
action-app-open-settings-file = Abrir settings.json

## Menu bar
action-app-hide = Ocultar Parla
action-app-hide-others = Ocultar otras
action-app-show-all = Mostrar todo
action-app-minimize = Minimizar
action-app-zoom = Ampliar
action-app-close-window = Cerrar ventana
action-app-bring-all-to-front = Traer todo al frente
action-app-open-docs = Documentación de Parla
action-app-open-keyboard-shortcuts = Atajos de teclado
action-app-report-issue = Informar de un problema
menu-settings = Ajustes…
menu-edit = Edición
menu-view = Visualización
menu-window = Ventana
menu-help = Ayuda
menu-undo = Deshacer
menu-redo = Rehacer
menu-cut = Cortar
menu-copy = Copiar
menu-paste = Pegar
menu-select-all = Seleccionar todo
shell-switch-to = Cambiar a { $workspace }
shell-dm-unsupported = Todavía no se puede escribir a { $person } desde aquí.
shell-dm-failed = No se pudo abrir una conversación con { $person }: { $reason }
shell-admin-failed = No ha funcionado: { $reason }
shell-join-failed = No te has podido unir al canal
shell-invite-title = Añadir personas a #{ $channel }
shell-invite-person-failed = { $person }: { $reason }
shell-leave-title = ¿Salir de #{ $channel }?
shell-leave-body = Puedes volver a unirte desde Explorar canales.
shell-leave-body-private = Es privado: necesitarás una invitación para volver.
shell-leave-ok = Salir
shell-archive-title = ¿Archivar #{ $channel }?
shell-archive-body = Nadie podrá publicar en él. Sus mensajes seguirán pudiéndose leer y buscar.
shell-archive-ok = Archivar

## Keyboard shortcuts (the sheet and the Settings Keymap page)
shortcuts-area-navigation = Navegación
shortcuts-area-messages = Mensajes
shortcuts-area-composer = Redactor
shortcuts-area-calls = Llamadas
shortcuts-area-app = App y ventanas
shortcuts-workspace-range = Cambiar al espacio de trabajo 1–9
shortcuts-area-text = Edición de texto
settings-keymap-subtitle = Cada acción tiene un atajo que puedes cambiar. Tus atajos están en keymap.json y se aplican sobre los predeterminados.
settings-keymap-open-file = Abrir keymap.json
settings-keymap-search = Buscar por acción o teclas
settings-keymap-count = { $shown } de { $total } acciones
settings-keymap-filter-all = Todas
settings-keymap-filter-changed = Cambiadas
settings-keymap-filter-conflicts = Conflictos · { $count }
settings-keymap-needs-modifier = Añade un modificador como Cmd, Ctrl o Alt: aquí una tecla sola se activaría mientras escribes.
settings-keymap-shadow = También ejecuta { $action }. Mientras { $context } tiene el foco, gana su atajo.
settings-keymap-col-action = Acción
settings-keymap-col-shortcut = Atajo
settings-keymap-col-context = Contexto
settings-keymap-col-source = Origen
settings-keymap-context-any = En cualquier lugar
settings-keymap-source-default = Predeterminado
settings-keymap-source-user = Tú
settings-keymap-unbound = Sin asignar
settings-keymap-edit-label = Cambiar el atajo de { $action }
settings-keymap-press-keys = Pulsa las teclas…
settings-keymap-save = Guardar
settings-keymap-cancel = Cancelar
settings-keymap-reset = Restablecer
settings-keymap-conflict = También ejecuta { $action } aquí. Al guardar, lo sustituye.
settings-keymap-footer = Intro guarda y Esc cancela; Tab no se puede grabar. Una segunda tecla crea una secuencia como Cmd-K Cmd-S. Los contextos se anidan: los atajos del redactor ganan a los globales mientras escribes.
settings-keymap-error-not-array = keymap.json no es un array JSON, así que no se guardó nada. Corrige el archivo o bórralo.

settings-logs-title = Registros y diagnóstico
settings-logs-desc = Los tokens, las cookies y el texto de los mensajes no entran en el registro ni en la exportación.
settings-log-level = Nivel de registro
settings-log-level-help = Cuánto escribe Parla en su registro. Se aplica al instante.
settings-log-error = Error
settings-log-info = Info
settings-log-debug = Depuración
settings-open-logs = Abrir la carpeta de registros
settings-open-logs-label = Registros
settings-open-logs-help = Muestra parla.log en tu gestor de archivos.
settings-export-diagnostics = Exportar…
settings-export-label = Diagnóstico redactado para un informe de error
settings-export-help = Guarda un archivo de texto con la versión, los ajustes, las últimas líneas del registro, los contadores y el último informe de fallos, y lo muestra. Léelo antes de compartirlo.
settings-error-export = No se pudo exportar el diagnóstico: { $reason }
lock-title = Parla está bloqueado
lock-detail = Parla cerró el almacén cifrado tras un rato sin uso. Desbloquéalo para abrirlo de nuevo con la clave de tu llavero.
lock-unlock = Desbloquear
lock-error = No se pudo leer la clave del almacén cifrado del llavero. Vuelve a pulsar Desbloquear.

code-viewer-meta = { $sender } en { $place } · { $time }
code-viewer-meta-direct = { $sender } · { $time }
code-viewer-jump = Ir al mensaje
code-viewer-wrap = Ajustar
code-viewer-line-numbers = Números de línea
code-viewer-copy = Copiar
code-viewer-copied = Copiado
code-viewer-download = Descargar
code-viewer-find-placeholder = Buscar en el archivo
code-viewer-find-count = { $index } de { $count }
code-viewer-find-count-capped = { $index } de { $count }+
code-viewer-find-none = Sin resultados
code-viewer-find-count-label = Coincidencia { $index } de { $count }
code-viewer-find-count-label-capped = Coincidencia { $index } de más de { $count }
code-viewer-region =
    { $count ->
        [one] Código, 1 línea
       *[other] Código, { $count } líneas
    }
code-viewer-find-prev = Coincidencia anterior
code-viewer-find-next = Coincidencia siguiente
code-viewer-find-close = Cerrar la búsqueda
code-viewer-loading = Cargando el archivo completo
code-viewer-failed = No se pudo cargar el archivo completo; se muestra la vista previa.
code-viewer-failed-empty = No se pudo cargar este archivo. Descárgalo.
code-viewer-too-large = Este archivo es demasiado grande para mostrarlo aquí. Descárgalo.
code-viewer-not-text = Este archivo no es texto. Descárgalo.
code-viewer-lines =
    { $count ->
        [one] 1 línea
       *[other] { $count } líneas
    }
