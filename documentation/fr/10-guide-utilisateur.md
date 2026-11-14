# 10. Guide utilisateur

> 🇬🇧 [English version](../en/10-user-guide.md)

Markoo garde synchronisés un dossier de votre base de connaissance (KB) et un dossier d'un dépôt Git. Ce guide couvre l'usage courant ; les autres pages expliquent le fonctionnement.

> **Gardez une sauvegarde de vos notes et commencez sur une copie.** Markoo est une version candidate. Il conserve ce qu'il écrase, mais une erreur dans le choix des dossiers copie beaucoup de fichiers.

## Installer

Systèmes pris en charge :

| Système | Application | Ligne de commande |
| --- | --- | --- |
| macOS 11 ou plus récent, Apple Silicon | ✅ | ✅ |
| macOS 11 ou plus récent, Intel | ❌ | ✅ |
| Linux x86_64 avec glibc 2.35 ou plus récent (Ubuntu 22.04, Debian 12 ou plus récents) | ✅ | ✅ |
| Windows 10 ou plus récent, x86_64 | ❌ | ✅ |

### Application (macOS)

Téléchargez le `.dmg` sur les [versions publiées](https://github.com/studoo-app/markoo/releases) et glissez Markoo dans Applications. L'app est construite pour les Mac Apple Silicon seulement ; sur un Mac Intel, utilisez la ligne de commande.

L'app est signée et notarisée par Apple : elle s'ouvre donc normalement. Elle contient la commande `markoo`, dans `/Applications/Markoo.app/Contents/MacOS/markoo`.

### Application (Linux)

Deux paquets sont sur la même page :

- **`.deb`** pour Debian, Ubuntu et leurs dérivées : `sudo apt install ./Markoo_<version>_amd64.deb`. Il installe l'app sous le nom `markoo-app`, avec une entrée dans le menu, et la commande dans `/usr/bin/markoo`. Il a besoin de WebKitGTK 4.1, qu'`apt` installe avec lui.
- **`.AppImage`** pour les autres distributions : rendez-le exécutable (`chmod +x Markoo-x86_64-unknown-linux-gnu.AppImage`) et lancez-le. L'AppImage se met à jour depuis l'app.

L'icône de la barre de menus demande un bureau qui gère les icônes de notification (AppIndicator). Sous GNOME, il faut l'extension AppIndicator.

### Ligne de commande seule

Téléchargez l'archive de votre système sur la même page et placez `markoo` dans votre `PATH`. Elle tourne sur macOS (Apple Silicon et Intel), Linux x86_64 et Windows x86_64. Sur macOS, elle est aussi signée et notarisée, mais un Mac doit pouvoir joindre Apple à son premier lancement pour le vérifier. Si le système la bloque, hors ligne par exemple, utilisez la copie incluse dans l'app ou retirez la marque de quarantaine :

```bash
xattr -d com.apple.quarantine ./markoo
```

Pour la compiler depuis les sources, avec Rust installé :

```bash
cargo install --path crates/markoo-cli --locked
```

## Créer votre première paire

Une **paire** relie un dossier de la KB à un dossier du dépôt. Choisissez des dossiers précis : une paire suit tout ce qu'ils contiennent.

Les paires sont regroupées par **projet** : le projet `Markoo`, par exemple, peut contenir une paire `documentation` et une paire `skill`. Vous nommez le projet à la création de sa première paire ; les suivantes le choisissent, ou en nomment un nouveau.

**Dans l'app**, cliquez sur **Ajouter un sync** et suivez les étapes. La première demande le projet : nommez-le pour votre première paire, puis choisissez-le dans la liste. **Lisez la dernière avant de créer la paire** : elle liste ce qui sera copié et dans quel sens. Elle signale les fichiers envoyés vers la KB que Git ne suit pas dans le dépôt : ils peuvent n'avoir aucun rapport avec le projet, par exemple si une paire précédente les y a laissés.

**En ligne de commande** :

```bash
markoo pair add clara --project Clara --kb ~/Obsidian/Knowledge/Projets/Clara --repo ~/code/clara/docs
```

Sans arguments, `markoo pair add` pose les mêmes questions dans le terminal, en commençant par le projet.

Regardez ensuite le plan, qui n'écrit rien :

```bash
markoo sync --pair clara --dry-run
```

Si c'est ce que vous attendez, synchronisez :

```bash
markoo sync --pair clara
```

## Regrouper les paires par projet

Ajoutez les paires suivantes d'un projet avec le même `--project`. L'orthographe déjà utilisée est conservée, donc `--project clara` rejoint `Clara` :

```bash
markoo pair add Skill --project Clara --kb ~/Obsidian/Knowledge/Skills/Clara --repo ~/code/clara/skill
```

`markoo pair list` affiche les paires projet par projet. Pour agir sur toutes les paires d'un projet d'un coup :

```bash
markoo status --project Clara
```

```bash
markoo sync --project Clara
```

Dans l'app, l'accueil affiche un bloc par projet, avec **Synchroniser le projet** et **Ajouter un sync**. Les paires créées avant les projets n'en ont pas et sont listées en dernier ; `markoo pair set clara project Clara` leur en donne un, ou change une paire de projet.

## Nommer et renommer une paire

Le nom d'une paire est un texte libre, comme `"Documentation technique"` ou `"Skill Claude (prod)"`, et deux projets peuvent avoir chacun une paire `Documentation`. Renommez une paire quand vous le souhaitez, dans son onglet **Général** ou avec :

```bash
markoo pair set clara name "Documentation Clara"
```

Renommer conserve l'historique, les conflits en attente et les hooks Git. Ils sont liés à l'**identifiant** de la paire, écrit une fois pour toutes à sa création : `clara` ici, `clara-skill` pour la paire `Skill` ci-dessus, `markoo-documentation` pour la paire `Documentation` de `Markoo`. `markoo pair show` et l'onglet **Général** l'affichent.

Dans une commande, désignez une paire par son identifiant, par `Projet/Nom` (`"Clara/Documentation Clara"`), ou par son nom seul quand aucun autre projet n'a de paire de ce nom ; sinon Markoo refuse et liste les paires concernées. **Utilisez l'identifiant dans les scripts** : il ne change pas quand la paire est renommée. Voir [Désigner une paire](06-configuration.md#désigner-une-paire).

> Tout ce que contient le dossier de la KB finit dans le dépôt, y compris les pièces jointes et les notes privées. Utilisez des motifs `exclude`, dans l'onglet **Filters** ou avec `markoo pair set`, avant de synchroniser vers un dépôt partagé.

## Choisir un sens

| Sens | Valeur | À utiliser quand |
|------|--------|------------------|
| KB ⇄ Repo | `both` | Vous écrivez des deux côtés |
| KB → Repo | `kb-to-repo` | La KB fait référence |
| KB ← Repo | `repo-to-kb` | Le dépôt fait référence |

Dans une paire à sens unique, une modification de la cible est signalée comme **divergence** et laissée telle quelle. Voir [Algorithme de synchronisation](03-algorithme-de-synchronisation.md).

## Régler les conflits

Quand les deux côtés ont changé les mêmes lignes, Markoo n'écrit rien pour ce fichier et attend votre décision. L'icône de la barre de menus passe à l'orange.

**Dans l'app**, ouvrez la paire, puis l'onglet **Conflicts** : il montre les versions KB, base et dépôt, et une zone de texte pour le résultat.

**En ligne de commande**, listez-les, puis réglez chacun :

```bash
markoo conflicts
```

```bash
markoo resolve architecture.md --keep kb
```

`--keep` accepte `kb`, `repo` ou `both`, et `--edit` ouvre le texte fusionné dans votre éditeur.

## Synchroniser automatiquement

| Moyen | Comment | Remarque |
|-------|---------|----------|
| App | Laissez-la tourner dans la barre de menus | Fermer la fenêtre ne l'arrête pas ; **Quit Markoo** l'arrête |
| Service (macOS) | `markoo daemon install` | Sans l'app ; `markoo daemon uninstall` le retire |
| Hooks Git | Onglet **Triggers**, ou `markoo hook install --pair clara` | L'installation ne synchronise rien : un hook agit la prochaine fois que Git le lance |
| MCP | `claude mcp add markoo -- markoo mcp` | Permet à Claude Code ou à un autre client d'utiliser Markoo |

L'intervalle se règle dans les **Settings** de l'app, ou dans `[schedule]` de `markoo.toml`. Lancez `markoo hook uninstall` avant de supprimer une paire, sinon les hooks restent dans le dépôt. Voir [Déclencheurs](05-declencheurs.md).

> Avec MCP, le contenu des conflits et les diffs sont envoyés au modèle que vous utilisez : le texte de vos notes quitte donc votre machine si ce modèle est distant.

## Retrouver vos fichiers

`markoo paths` affiche le fichier de configuration et le dossier de données utilisés. Sur macOS, les deux sont dans `~/Library/Application Support/dev.markoo.markoo/`.

| Dans le dossier de données | Contenu |
|----------------------------|---------|
| `backups/<identifiant>/` | Ce que Markoo a écrasé, par synchro |
| `trash/<identifiant>/` | Ce que Markoo a supprimé, par synchro |
| `hooks.log` | Erreurs des hooks Git |
| `daemon.log` | Sortie du service `launchd` |

### Revenir en arrière

L'onglet **History** de l'app liste ce que chaque synchro a changé. Ouvrez une synchro puis choisissez **Revenir en arrière…** ; en ligne de commande :

```bash
markoo history clara
markoo revert clara 12
```

Markoo remet les fichiers que cette synchro a touchés : un fichier écrasé retrouve sa version précédente, un fichier créé part à la corbeille, un fichier supprimé revient et un fichier déplacé retourne à sa place. Un fichier modifié depuis n'est pas touché, et il est signalé. La synchro suivante reporte les anciennes versions de l'autre côté.

Le retour en arrière est lui-même enregistré dans l'historique, avec ses propres sauvegardes : il peut être annulé à son tour. Limites :

- Les synchros enregistrées avant la version qui a ajouté le retour en arrière ne peuvent pas être annulées ; copiez les fichiers depuis les dossiers ci-dessus.
- Pour une synchro qui a fusionné un fichier des deux côtés, les deux versions d'avant reviennent, et la synchro suivante signale un conflit.
- Dans une paire à sens unique, le retour en arrière d'une copie vers la cible devient une divergence si la paire protège sa cible ; il est refusé si elle écrase les changements de sa cible, car la synchro suivante réécrirait les mêmes fichiers.

## Arrêter d'utiliser Markoo

Supprimer une paire garde tous les fichiers des deux côtés, et ceux qu'elle a copiés restent en place. **Supprimez la paire avant d'effacer ses fichiers** : sinon, la synchro suivante prend la suppression pour un changement et la reporte de l'autre côté.

1. Lancez `markoo hook uninstall --pair clara` si vous avez installé des hooks.
2. Lancez `markoo pair remove clara`, ou utilisez l'onglet **General**.
3. Lancez `markoo daemon uninstall` si vous avez installé le service.
4. Retirez l'app, puis le dossier indiqué par `markoo paths`.

## Limites connues

- L'app ne tourne que sur macOS.
- Il n'y a pas encore de formule Homebrew.
- L'intervalle s'applique à toutes les paires.
- Les options non implémentées sont acceptées dans `markoo.toml` et ignorées avec un avertissement.

Le [changelog](../../CHANGELOG.md) les liste pour chaque version.
