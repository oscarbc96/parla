import type { Ui } from "@/i18n/ui";

export const ca: Ui = {
  nav: {
    main: "Principal",
    docs: "Documentació",
    changelog: "Novetats",
    download: "Baixades",
    github: "GitHub",
  },
  footer: {
    license: "Parla és codi obert sota la MPL-2.0.",
    source: "Codi a GitHub",
    privacy: "Privadesa",
    llms: "llms.txt",
    colophon: "Aquest lloc està fet amb Astro i Fumadocs.",
  },
  docsTitle: "Documentació",
  language: "Llengua",
  home: {
    title: "Parla: un client d'escriptori natiu i ràpid per a Slack",
    description:
      "Client d'escriptori no oficial i de codi obert per a Slack, escrit en Rust sobre GPUI. Poca memòria, diversos espais de treball, teclat primer i servidor MCP local.",
    kicker: "Client d'escriptori no oficial",
    heading: "Digues-ho i prou.",
    headingAccent: "Just parla.",
    lede: "Parla és un client d'escriptori natiu i ràpid per a Slack. Escrit en Rust sobre GPUI i dibuixat de manera nativa, Parla no porta cap webview resident ni entorn d'execució de JavaScript. Tots els espais de treball continuen amb la sessió iniciada, tot es fa amb el teclat i un servidor MCP local permet que els agents d'IA de la teva màquina llegeixin les teves sessions i hi escriguin només amb confirmació.",
    docsButton: "Documentació",
    sourceButton: "Codi a GitHub",
    status: "En desenvolupament",
    statusText: "Compilacions autosignades de macOS a GitHub Releases.",
    statusLink: "Compila'l des del codi font",
    platform:
      "macOS 14 o posterior, Apple silicon i Intel. Windows i Linux: compila'l des del codi font, els instal·ladors arribaran més endavant.",
    heroCaption:
      "Un fil obert al costat de la seva conversa, amb la barra lateral a l'esquerra.",
    featuresLabel: "Què es proposa Parla",
    featuresHeading: "Fet per a qui viu al xat.",
    features: [
      {
        title: "Velocitat nativa",
        text: "GPUI dibuixa a la GPU. Els missatges, el canvi de conversa i la cerca no esperen cap motor de navegador.",
        linkText: "Instal·la Parla",
      },
      {
        title: "Poca memòria",
        text: "Un procés, un fil d'interfície i una memòria cau local xifrada amb la mida que triïs.",
        linkText: "Sense connexió i memòria cau",
      },
      {
        title: "Diversos espais de treball",
        text: "Tots els espais continuen amb la sessió iniciada alhora, amb un selector, insígnies per espai i una única vista de no llegits.",
        linkText: "Espais i barra lateral",
      },
      {
        title: "Pensat per al teclat",
        text: "Selector ràpid, paleta d'ordres, salt al següent no llegit i un mapa de tecles en JSONC on cada acció té un nom estable.",
        linkText: "Ajustos i mapa de tecles",
      },
      {
        title: "Huddles",
        text: "Els huddles actius surten a la barra lateral i en una barra de trucada. Un huddle entrant sona i es pot acceptar o rebutjar.",
        linkText: "Huddles al xat",
      },
      {
        title: "Fitxers i vistes prèvies",
        text: "Les imatges i el vídeo es previsualitzen al xat, els PDF mostren la imatge de vista prèvia que aporta el servei, els enviaments admeten fitxers de fins a 1 GB i els clips s'obren a l'app.",
        linkText: "Fitxers i clips",
      },
      {
        title: "Notificacions discretes",
        text: "Pausa-ho tot, tria un nivell per conversa i mantén en silenci els xats silenciats. La notificació diu qui i on.",
        linkText: "Regles de notificació",
      },
      {
        title: "Servidor MCP local",
        text: "Els agents d'IA de la mateixa màquina llegeixen les teves sessions i escriuen només amb confirmació. Desactivat per defecte i només local.",
      },
      {
        title: "Codi obert",
        text: "MPL-2.0, un únic repositori. Les compilacions autosignades de macOS són a GitHub Releases. El SDK de connectors és Apache-2.0 OR MIT.",
        linkText: "Llegeix el codi",
      },
    ],
    downloadHeading: "Prova Parla al teu Mac.",
    downloadText:
      "La primera versió és per a macOS: un dmg universal autosignat a GitHub Releases. Després arriben Windows i Linux. També el pots compilar des del codi font.",
    themesLabel: "Fes-lo teu",
    themesTitle: "Els teus colors, en un fitxer que pots compartir.",
    themesTextBefore:
      "Els temes són fitxers JSON amb la mateixa forma general que els temes de Zed. Deixa'n un a ",
    themesTextAfter:
      " i es recarrega en calent. Canvia un sol token o tota la paleta i envia el fitxer al teu equip.",
    themesLight: "Parla Light",
    themesDark: "Parla Dark",
    themesDefault: "Predeterminat",
    themesPresets: "Accents predefinits",
    themesFile: "themes/priya.json",
    themesLink: "Aparença a la documentació",
    extLabel: "Amplia'l",
    extTitle: "Amplia'l sense lliurar la teva sessió.",
    extText:
      "Previst: connectors que s'executen com a WebAssembly en un entorn aïllat amb els permisos que declaren i mai veuen el token de la teva sessió. Un servidor MCP local permet que els agents d'IA d'aquesta màquina llegeixin i escriguin només després que ho confirmis.",
    extPluginsTitle: "Connectors",
    extPluginsText:
      "Previst: ordres, ordres amb barra, accions de missatge, panells i finestres, dibuixats amb components natius. SDK sota Apache-2.0 OR MIT.",
    extMcpTitle: "Servidor MCP local",
    extMcpText:
      "Desactivat per defecte i només local. Cada escriptura es confirma per defecte i queda registrada en un registre d'auditoria.",
    extKeymapTitle: "Mapa de tecles",
    extKeymapText:
      "Un mapa de tecles JSONC on cada acció té un nom estable, perquè connectors i persones assignin el mateix.",
    buildButton: "Compilar des del codi font",
    releasesButton: "Versions a GitHub",
    changelogButton: "Novetats",
    allDownloads: "Totes les opcions de baixada",
    heroAlt:
      "Un fil obert al costat de la seva conversa a Parla: el missatge original i dues respostes, amb la barra lateral a l'esquerra.",
    galleryLabel: "Parla d'un cop d'ull",
    gallery: [
      {
        shot: "search",
        title: "Cerca que es queda a la conversa",
        text: "Prem cmd-shift-f per cercar als missatges desats en aquest dispositiu i després al proveïdor. Filtres com in:, from: i has: funcionen com a paraules.",
        alt: "El panell de cerca al costat d'una conversa: resultats de 'cache' amb les paraules coincidents ressaltades.",
      },
      {
        shot: "switcher",
        title: "Salta a qualsevol lloc amb el teclat",
        text: "El selector ràpid troba una conversa de qualsevol espai de treball amb sessió iniciada mentre escrius.",
        alt: "El selector ràpid obert sobre una conversa, amb els canals que coincideixen amb 'gen' en dos espais de treball.",
      },
      {
        shot: "huddle",
        title: "Huddles en directe d'un cop d'ull",
        text: "Un punt verd a la barra lateral, una barra amb qui hi participa i un botó Uneix-te, i targetes a la conversa.",
        alt: "Una conversa amb un huddle en curs: un punt verd a la barra lateral, una barra amb les persones que hi participen i un botó Uneix-te, i targetes de huddle a la conversa.",
      },
    ],
    connectLabel: "Com es connecta",
    connectHeading: "La teva pròpia sessió. Sense servidor al mig.",
    connectText:
      "Parla inicia la sessió amb el testimoni i la galeta de la teva pròpia sessió web de Slack, així que un sol inici de sessió cobreix tots els espais que ja tinguis oberts en aquella sessió. No cal crear cap app de Slack ni s'executa cap servidor del projecte. És un enfocament no oficial amb riscos reals, així que llegeix abans la guia d'inici de sessió.",
    connectBullet1:
      "Magatzem local xifrat i una memòria cau amb la mida que triïs",
    connectBullet2: "Credencials al clauer del sistema",
    connectBullet3:
      "MPL-2.0, un sol repositori i versions autosignades a GitHub",
    connectSignIn: "Llegeix la guia d'inici de sessió",
    connectPrivacy: "Notes de privadesa",
  },
  changelog: {
    title: "Novetats | Parla",
    description:
      "Notes de cada versió publicada de Parla, preses de les versions de GitHub.",
    heading: "Novetats",
    notesNote:
      "Les notes de cada versió es publiquen en anglès a les versions de GitHub.",
    empty: "Encara no hi ha versions",
    emptyText:
      "Encara no s'ha publicat res. Les versions apareixeran aquí tan bon punt surti la primera.",
    prerelease: "Versió preliminar",
    unreleased: {
      heading: "Sense publicar",
      badge: "Encara no publicat",
      version: "Versió proposada {version}",
      note: "Aquests canvis són a la branca principal i encara no formen part de cap versió. Poden canviar abans que surti la següent.",
    },
    newer: "Més recents",
    older: "Anteriors",
    dateLocale: "ca",
  },
  downloads: {
    title: "Baixades | Parla",
    description:
      "Baixa la compilació autosignada per a macOS o compila Parla des del codi font. Després arriben Windows i Linux.",
    heading: "Baixades",
    lede: "Baixa la compilació autosignada per a macOS o compila Parla des del codi font.",
    macTitle: "macOS",
    downloadMac: "Baixa per a macOS",
    macRequirements: "macOS 14 o posterior, Apple silicon i Intel",
    latest: "Última",
    signed: "Autosignada",
    shaLabel: "SHA-256",
    copy: "Copia",
    copied: "Copiat",
    checksumLink: "Suma de comprovació (SHA256SUMS)",
    attestationLabel: "Verifica la baixada",
    installTitle: "Instal·lar",
    installStep1: {
      title: "Obre el .dmg",
      text: "Arrossega Parla a Aplicacions.",
    },
    installStep2: {
      title: "Obre Parla",
      text: "Comprova les actualitzacions pel seu compte; ho pots desactivar als ajustos.",
    },
    installStep3: {
      title: "Permet el primer arrencament",
      text: "macOS ho bloqueja una vegada. A macOS 15 o posterior, fes servir Configuració del Sistema, Privadesa i seguretat, Obre-ho igualment. A macOS 14, fes Control-clic a Parla i tria Obre.",
    },
    installStep4: {
      title: "Inicia sessió",
      text: "Llegeix abans la guia d'inici de sessió: Parla fa servir la teva pròpia sessió web.",
    },
    installLink: "Guia d'instal·lació",
    otherPlatformsTitle: "Windows i Linux",
    otherPlatformsText:
      "Compilar des del codi font; instal·ladors més endavant.",
    later: "Més endavant",
    noRelease: "Encara no hi ha cap versió",
    noReleaseText: "Encara no hi ha cap versió publicada per baixar.",
    releasesLink: "Versions a GitHub",
    buildTitle: "Compilar des del codi font",
    buildText:
      "Pots compilar i executar Parla tu mateix amb mise i la cadena d'eines de Rust.",
    buildLink: "Guia d'instal·lació",
    whatsNew: "Novetats de {version}",
    notesText:
      "Les notes de la versió venen de les versions de GitHub, en anglès.",
    fullChangelog: "Novetats completes",
    olderTitle: "Versions anteriors",
    tableVersion: "Versió",
    tableDate: "Data",
    tableDownload: "Baixada",
    tableChecksum: "Suma de comprovació",
    allReleases: "Totes les versions a GitHub",
  },
  notFound: {
    title: "Pàgina no trobada | Parla",
    description:
      "Aquesta pàgina no existeix. Torna a la pàgina d'inici de Parla o a la documentació.",
    heading: "Pàgina no trobada",
    text: "La pàgina que cerques no existeix o ha canviat de lloc.",
    home: "Torna a l'inici",
  },
  docsVersions: {
    label: "Versió de la documentació",
    search: "Cerca versions",
    noMatch: "Cap versió coincideix",
    latest: "darrera",
    unreleased: "Sense publicar",
    searching: "Cercant a {version}",
    oldText: "Aquesta documentació és de Parla {version}.",
    oldLink: "Veure la darrera versió.",
    nextText: "Aquesta documentació descriu canvis sense publicar.",
    nextLink: "Veure la darrera versió publicada.",
  },
  starsLabel: "GitHub, {count} estrelles",
  // Fumadocs UI keys are the English text followed by its "(note)" suffixes.
  translations: {
    "Choose a language(language switcher)": "Tria una llengua",
    "Choose a language(language switcher)(aria-label)": "Tria una llengua",
    "Search(search dialog)": "Cerca",
    "Search(search trigger)": "Cerca",
    "Open Search(search trigger)(aria-label)": "Obre la cerca",
    "Close Search(search dialog)(aria-label)": "Tanca la cerca",
    "On this page(table of contents)": "En aquesta pàgina",
    "No Headings(table of contents)": "Sense encapçalaments",
    "Next Page(pagination)": "Pàgina següent",
    "Previous Page(pagination)": "Pàgina anterior",
    "Toggle Menu(mobile menu)(aria-label)": "Mostra o amaga el menú",
    "Toggle Theme(theme switcher)(aria-label)": "Canvia el tema",
    "Light(theme switcher)(aria-label)": "Clar",
    "Dark(theme switcher)(aria-label)": "Fosc",
    "System(theme switcher)(aria-label)": "Sistema",
    "Open Sidebar(sidebar)(aria-label)": "Obre la barra lateral",
    "Close Sidebar(sidebar)(aria-label)": "Tanca la barra lateral",
    "Collapse Sidebar(sidebar)(aria-label)": "Plega la barra lateral",
    "Skip to Content(skip link)": "Salta al contingut",
    "Copy Text(code block)(aria-label)": "Copia el text",
    "Copied Text(code block)(aria-label)": "Text copiat",
  },
};
