import type { Ui } from "@/i18n/ui";

export const es: Ui = {
  nav: {
    main: "Principal",
    docs: "Documentación",
    changelog: "Novedades",
    download: "Descargas",
    github: "GitHub",
  },
  footer: {
    license: "Parla es código abierto bajo MPL-2.0.",
    source: "Código en GitHub",
    privacy: "Privacidad",
    llms: "llms.txt",
    colophon: "Este sitio está hecho con Astro y Fumadocs.",
  },
  docsTitle: "Documentación",
  language: "Idioma",
  home: {
    title: "Parla: un cliente de escritorio nativo y rápido para Slack",
    description:
      "Cliente de escritorio no oficial y de código abierto para Slack, escrito en Rust sobre GPUI. Poca memoria, varios espacios, teclado primero y servidor MCP local.",
    kicker: "Cliente de escritorio no oficial",
    heading: "Dilo sin más.",
    headingAccent: "Just parla.",
    lede: "Parla es un cliente de escritorio nativo y rápido para Slack. Escrito en Rust sobre GPUI y dibujado de forma nativa, Parla no lleva webview residente ni entorno de ejecución de JavaScript. Todos los espacios de trabajo siguen con la sesión iniciada, todo se maneja con el teclado y un servidor MCP local permite que los agentes de IA de tu máquina lean tus sesiones y escriban en ellas solo con confirmación.",
    docsButton: "Documentación",
    sourceButton: "Código en GitHub",
    status: "En desarrollo",
    statusText: "Compilaciones autofirmadas de macOS en GitHub Releases.",
    statusLink: "Compílalo desde el código fuente",
    platform:
      "macOS 14 o posterior, Apple silicon e Intel. Windows y Linux: compílalo desde el código fuente, los instaladores llegarán más adelante.",
    heroCaption:
      "Un hilo abierto junto a su conversación, con la barra lateral a la izquierda.",
    featuresLabel: "Qué se propone Parla",
    featuresHeading: "Hecho para quien vive en el chat.",
    features: [
      {
        title: "Velocidad nativa",
        text: "GPUI dibuja en la GPU. Los mensajes, el cambio de conversación y la búsqueda no esperan a ningún motor de navegador.",
        linkText: "Instalar Parla",
      },
      {
        title: "Poca memoria",
        text: "Un proceso, un hilo de interfaz y una caché local cifrada con el tamaño que elijas.",
        linkText: "Sin conexión y caché",
      },
      {
        title: "Varios espacios de trabajo",
        text: "Todos los espacios siguen con la sesión iniciada a la vez, con un selector, insignias por espacio y una única vista de no leídos.",
        linkText: "Espacios y barra lateral",
      },
      {
        title: "Pensado para el teclado",
        text: "Selector rápido, paleta de comandos, salto al siguiente no leído y un mapa de teclas en JSONC donde cada acción tiene un nombre estable.",
        linkText: "Ajustes y mapa de teclas",
      },
      {
        title: "Huddles",
        text: "Los huddles activos aparecen en la barra lateral y en una barra de llamada. Un huddle entrante suena y se puede aceptar o rechazar.",
        linkText: "Huddles en el chat",
      },
      {
        title: "Archivos y vistas previas",
        text: "Las imágenes y el vídeo se previsualizan en el chat, los PDF muestran la imagen de vista previa que aporta el servicio, los envíos admiten archivos de hasta 1 GB y los clips se abren en la app.",
        linkText: "Archivos y clips",
      },
      {
        title: "Notificaciones discretas",
        text: "Pausa todo, elige un nivel por conversación y mantén en silencio los chats silenciados. La notificación dice quién y dónde.",
        linkText: "Reglas de notificación",
      },
      {
        title: "Servidor MCP local",
        text: "Los agentes de IA de la misma máquina leen tus sesiones y escriben solo con confirmación. Desactivado por defecto y solo local.",
      },
      {
        title: "Código abierto",
        text: "MPL-2.0, un único repositorio. Las compilaciones autofirmadas de macOS están en GitHub Releases. El SDK de plugins es Apache-2.0 OR MIT.",
        linkText: "Leer el código",
      },
    ],
    downloadHeading: "Prueba Parla en tu Mac.",
    downloadText:
      "La primera versión es para macOS: un dmg universal autofirmado en GitHub Releases. Después llegan Windows y Linux. También puedes compilarlo desde el código fuente.",
    themesLabel: "Hazlo tuyo",
    themesTitle: "Tus colores, en un archivo que puedes compartir.",
    themesTextBefore:
      "Los temas son archivos JSON con la misma forma general que los temas de Zed. Deja uno en ",
    themesTextAfter:
      " y se recarga en caliente. Cambia un solo token o toda la paleta y envía el archivo a tu equipo.",
    themesLight: "Parla Light",
    themesDark: "Parla Dark",
    themesDefault: "Predeterminado",
    themesPresets: "Acentos predefinidos",
    themesFile: "themes/priya.json",
    themesLink: "Apariencia en la documentación",
    extLabel: "Amplíalo",
    extTitle: "Amplíalo sin entregar tu sesión.",
    extText:
      "Previsto: plugins que se ejecutan como WebAssembly en un entorno aislado con los permisos que declaran y nunca ven el token de tu sesión. Un servidor MCP local permite que los agentes de IA de esta máquina lean y escriban solo después de que lo confirmes.",
    extPluginsTitle: "Plugins",
    extPluginsText:
      "Previsto: comandos, comandos con barra, acciones de mensaje, paneles y ventanas, dibujados con componentes nativos. SDK bajo Apache-2.0 OR MIT.",
    extMcpTitle: "Servidor MCP local",
    extMcpText:
      "Desactivado por defecto y solo local. Cada escritura se confirma por defecto y queda registrada en un registro de auditoría.",
    extKeymapTitle: "Mapa de teclas",
    extKeymapText:
      "Un mapa de teclas JSONC donde cada acción tiene un nombre estable, para que plugins y personas asignen lo mismo.",
    buildButton: "Compilar desde el código fuente",
    releasesButton: "Versiones en GitHub",
    changelogButton: "Novedades",
    allDownloads: "Todas las opciones de descarga",
    heroAlt:
      "Un hilo abierto junto a su conversación en Parla: el mensaje original y dos respuestas, con la barra lateral a la izquierda.",
    galleryLabel: "Parla de un vistazo",
    gallery: [
      {
        shot: "search",
        title: "Búsqueda que se queda en la conversación",
        text: "Pulsa cmd-shift-f para buscar en los mensajes guardados en este dispositivo y después en el proveedor. Filtros como in:, from: y has: funcionan como palabras.",
        alt: "El panel de búsqueda junto a una conversación: resultados de 'cache' con las palabras coincidentes resaltadas.",
      },
      {
        shot: "switcher",
        title: "Salta a cualquier sitio con el teclado",
        text: "El selector rápido encuentra una conversación de cualquier espacio de trabajo con sesión iniciada mientras escribes.",
        alt: "El selector rápido abierto sobre una conversación, con los canales que coinciden con 'gen' en dos espacios de trabajo.",
      },
      {
        shot: "huddle",
        title: "Huddles en directo de un vistazo",
        text: "Un punto verde en la barra lateral, una barra con quién participa y un botón Unirse, y tarjetas en la conversación.",
        alt: "Una conversación con un huddle en curso: un punto verde en la barra lateral, una barra con las personas que participan y un botón Unirse, y tarjetas de huddle en la conversación.",
      },
    ],
    connectLabel: "Cómo se conecta",
    connectHeading: "Tu propia sesión. Sin servidor en medio.",
    connectText:
      "Parla inicia sesión con el token y la cookie de tu propia sesión web de Slack, así que un solo inicio de sesión cubre todos los espacios que ya tengas abiertos en esa sesión. No hay que crear ninguna app de Slack ni se ejecuta ningún servidor del proyecto. Es un enfoque no oficial con riesgos reales, así que lee antes la guía de inicio de sesión.",
    connectBullet1:
      "Almacén local cifrado y una caché con el tamaño que elijas",
    connectBullet2: "Credenciales en el llavero del sistema",
    connectBullet3:
      "MPL-2.0, un solo repositorio y versiones autofirmadas en GitHub",
    connectSignIn: "Leer la guía de inicio de sesión",
    connectPrivacy: "Notas de privacidad",
  },
  changelog: {
    title: "Novedades | Parla",
    description:
      "Notas de cada versión publicada de Parla, tomadas de las versiones de GitHub.",
    heading: "Novedades",
    notesNote:
      "Las notas de cada versión se publican en inglés en las versiones de GitHub.",
    empty: "Todavía no hay versiones",
    emptyText:
      "Aún no se ha publicado nada. Las versiones aparecerán aquí en cuanto salga la primera.",
    prerelease: "Versión preliminar",
    unreleased: {
      heading: "Sin publicar",
      badge: "Aún no publicado",
      version: "Versión propuesta {version}",
      note: "Estos cambios están en la rama principal y todavía no forman parte de ninguna versión. Pueden cambiar antes de que salga la siguiente.",
    },
    newer: "Más recientes",
    older: "Anteriores",
    dateLocale: "es-ES",
  },
  downloads: {
    title: "Descargas | Parla",
    description:
      "Descarga la compilación autofirmada para macOS o compila Parla desde el código fuente. Después llegan Windows y Linux.",
    heading: "Descargas",
    lede: "Descarga la compilación autofirmada para macOS o compila Parla desde el código fuente.",
    macTitle: "macOS",
    downloadMac: "Descargar para macOS",
    macRequirements: "macOS 14 o posterior, Apple silicon e Intel",
    latest: "Última",
    signed: "Autofirmada",
    shaLabel: "SHA-256",
    copy: "Copiar",
    copied: "Copiado",
    checksumLink: "Suma de comprobación (SHA256SUMS)",
    attestationLabel: "Verificar la descarga",
    installTitle: "Instalar",
    installStep1: {
      title: "Abre el .dmg",
      text: "Arrastra Parla a Aplicaciones.",
    },
    installStep2: {
      title: "Abre Parla",
      text: "Comprueba las actualizaciones por su cuenta; puedes desactivarlo en los ajustes.",
    },
    installStep3: {
      title: "Permite el primer arranque",
      text: "macOS lo bloquea una vez. En macOS 15 o posterior, usa Ajustes del Sistema, Privacidad y seguridad, Abrir de todos modos. En macOS 14, haz Control-clic en Parla y elige Abrir.",
    },
    installStep4: {
      title: "Inicia sesión",
      text: "Lee antes la guía de inicio de sesión: Parla usa tu propia sesión web.",
    },
    installLink: "Guía de instalación",
    otherPlatformsTitle: "Windows y Linux",
    otherPlatformsText:
      "Compilar desde el código fuente; instaladores más adelante.",
    later: "Más adelante",
    noRelease: "Todavía no hay ninguna versión",
    noReleaseText: "Aún no hay ninguna versión publicada para descargar.",
    releasesLink: "Versiones en GitHub",
    buildTitle: "Compilar desde el código fuente",
    buildText:
      "Puedes compilar y ejecutar Parla tú mismo con mise y la cadena de herramientas de Rust.",
    buildLink: "Guía de instalación",
    whatsNew: "Novedades de {version}",
    notesText:
      "Las notas de la versión vienen de las versiones de GitHub, en inglés.",
    fullChangelog: "Novedades completas",
    olderTitle: "Versiones anteriores",
    tableVersion: "Versión",
    tableDate: "Fecha",
    tableDownload: "Descarga",
    tableChecksum: "Suma de comprobación",
    allReleases: "Todas las versiones en GitHub",
  },
  notFound: {
    title: "Página no encontrada | Parla",
    description:
      "Esta página no existe. Vuelve a la página de inicio de Parla o a la documentación.",
    heading: "Página no encontrada",
    text: "La página que buscas no existe o ha cambiado de sitio.",
    home: "Volver al inicio",
  },
  docsVersions: {
    label: "Versión de la documentación",
    search: "Buscar versiones",
    noMatch: "Ninguna versión coincide",
    latest: "última",
    unreleased: "Sin publicar",
    searching: "Buscando en {version}",
    oldText: "Esta documentación es de Parla {version}.",
    oldLink: "Ver la última versión.",
    nextText: "Esta documentación describe cambios sin publicar.",
    nextLink: "Ver la última versión publicada.",
  },
  starsLabel: "GitHub, {count} estrellas",
  // Fumadocs UI keys are the English text followed by its "(note)" suffixes.
  translations: {
    "Choose a language(language switcher)": "Elige un idioma",
    "Choose a language(language switcher)(aria-label)": "Elige un idioma",
    "Search(search dialog)": "Buscar",
    "Search(search trigger)": "Buscar",
    "Open Search(search trigger)(aria-label)": "Abrir búsqueda",
    "Close Search(search dialog)(aria-label)": "Cerrar búsqueda",
    "On this page(table of contents)": "En esta página",
    "No Headings(table of contents)": "Sin encabezados",
    "Next Page(pagination)": "Página siguiente",
    "Previous Page(pagination)": "Página anterior",
    "Toggle Menu(mobile menu)(aria-label)": "Alternar menú",
    "Toggle Theme(theme switcher)(aria-label)": "Cambiar tema",
    "Light(theme switcher)(aria-label)": "Claro",
    "Dark(theme switcher)(aria-label)": "Oscuro",
    "System(theme switcher)(aria-label)": "Sistema",
    "Open Sidebar(sidebar)(aria-label)": "Abrir barra lateral",
    "Close Sidebar(sidebar)(aria-label)": "Cerrar barra lateral",
    "Collapse Sidebar(sidebar)(aria-label)": "Contraer barra lateral",
    "Skip to Content(skip link)": "Saltar al contenido",
    "Copy Text(code block)(aria-label)": "Copiar texto",
    "Copied Text(code block)(aria-label)": "Texto copiado",
  },
};
