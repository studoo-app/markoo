# Glossaire

> 🇬🇧 [English version](../en/glossary.md)

| Terme | Définition |
|-------|------------|
| **Base de connaissance (KB)** | Dossier de notes Markdown géré par un outil (par ex. Obsidian) ou simple dossier Markdown : l'un des deux côtés d'une paire. Son type (`kb_type`) détermine l'adaptateur utilisé. Ne jamais l'abréger en « base », qui désigne l'ancêtre de fusion. Voir [Bases de connaissance](09-bases-de-connaissance.md). |
| **Vault** | Une base de connaissance Obsidian : dossier racine qui contient `.obsidian/`. |
| **`kb_type`** | Type d'une KB dans `markoo.toml` : `markdown` ou `obsidian` en v1 ; `logseq`, `foam`, `dendron` après la v1. Détecté automatiquement s'il est absent. |
| **Adaptateur** | Implémentation du trait `KbAdapter` pour un type de KB : détection, organisation (notes, pièces jointes, exclusions), dialecte et résolution des noms. Voir [Modèle d'adaptateur](09-bases-de-connaissance.md#modèle-dadaptateur). |
| **Dialecte** | Variante de Markdown propre à un côté (wikilinks et callouts Obsidian, `gfm` côté repo…), convertie vers et depuis la forme canonique par `to_canonical` / `from_canonical`. |
| **Repo** | Le côté dépôt Git d'une paire : un dossier dans un clone local. |
| **Paire** | Association d'un dossier de la KB et d'un dossier du repo, synchronisés ensemble. |
| **Base** | Contenu d'un fichier lors de la dernière synchro réussie, stocké dans `state.db`. Sert d'ancêtre commun à la fusion ; stockée en forme canonique. |
| **Forme canonique** | GitHub Flavored Markdown standard avec liens relatifs, plus des commentaires invisibles `<!--markoo:…-->` pour ce que GFM ne sait pas représenter. Chaque adaptateur convertit son dialecte vers et depuis cette forme ; c'est sur elle qu'on compare, qu'on fusionne et qu'on stocke la base. |
| **Transformation** | Paire de fonctions `to_canonical` / `from_canonical` qui convertit une construction Markdown (liens, callouts…) d'un dialecte vers la forme canonique et retour. |
| **Fusion 3-way** | Fusion de deux versions modifiées à partir de leur ancêtre commun (la base). |
| **Hunk** | Bloc de lignes modifiées dans un diff. |
| **Conflit** | Hunks des deux côtés qui touchent les mêmes lignes de la base avec un résultat différent. |
| **Plan** | Liste des actions qu'une synchro effectuerait, calculée avant toute écriture. |
| **Dry-run** | Calcul et affichage du plan sans rien appliquer. |
| **Déclencheur** | Ce qui lance une synchro : intervalle, hook Git, API, MCP, CLI ou app. |
| **Daemon** | Processus Markoo en tâche de fond : planificateur, API et MCP. |
| **MCP** | *Model Context Protocol* : protocole qui permet à un assistant IA (Claude) d'appeler les outils de Markoo. |
| **Wikilink** | Lien Obsidian `[[Nom de note]]`. |
| **Embed** | Inclusion Obsidian `![[fichier]]` (image ou note). |
| **Callout** | Encadré Obsidian `> [!type]`, équivalent des *alerts* GitHub. |
