# 10. Guide utilisateur

> 🇬🇧 [English version](../en/10-user-guide.md)

Markoo garde synchronisés un dossier de votre base de connaissance (KB) et un dossier d'un dépôt Git. Ce guide couvre l'usage courant ; les autres pages expliquent le fonctionnement.

> **Gardez une sauvegarde de vos notes et commencez sur une copie.** Markoo est une version candidate. Il conserve ce qu'il écrase, mais une erreur dans le choix des dossiers copie beaucoup de fichiers.

## Installer

### Application (macOS)

Téléchargez le `.dmg` correspondant à votre Mac sur les [versions publiées](https://github.com/studoo-app/markoo/releases) (`aarch64` pour Apple Silicon, `x64` pour Intel) et glissez Markoo dans Applications.

L'app est signée et notarisée par Apple : elle s'ouvre donc normalement. Elle contient la commande `markoo`, dans `/Applications/Markoo.app/Contents/MacOS/markoo`.

### Ligne de commande seule

Téléchargez l'archive de votre système sur la même page et placez `markoo` dans votre `PATH`. Elle tourne sur macOS, Linux et Windows. Sur macOS, elle est aussi signée et notarisée, mais un Mac doit pouvoir joindre Apple à son premier lancement pour le vérifier. Si le système la bloque, hors ligne par exemple, utilisez la copie incluse dans l'app ou retirez la marque de quarantaine :

```bash
xattr -d com.apple.quarantine ./markoo
```

Pour la compiler depuis les sources, avec Rust installé :

```bash
cargo install --path crates/markoo-cli --locked
```

## Créer votre première paire

Une **paire** relie un dossier de la KB à un dossier du dépôt. Choisissez des dossiers précis : une paire suit tout ce qu'ils contiennent.

**Dans l'app**, cliquez sur **New pair** et suivez les étapes. **Lisez la dernière avant de créer la paire** : elle liste ce qui sera copié et dans quel sens. Elle signale les fichiers envoyés vers la KB que Git ne suit pas dans le dépôt : ils peuvent n'avoir aucun rapport avec le projet, par exemple si une paire précédente les y a laissés.

**En ligne de commande** :

```bash
markoo pair add clara --kb ~/Obsidian/Knowledge/Projets/Clara --repo ~/code/clara/docs
```

Regardez ensuite le plan, qui n'écrit rien :

```bash
markoo sync --pair clara --dry-run
```

Si c'est ce que vous attendez, synchronisez :

```bash
markoo sync --pair clara
```

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
| `backups/<paire>/` | Ce que Markoo a écrasé, par synchro |
| `trash/<paire>/` | Ce que Markoo a supprimé, par synchro |
| `hooks.log` | Erreurs des hooks Git |
| `daemon.log` | Sortie du service `launchd` |

**Il n'y a pas encore de retour en arrière automatique.** Pour récupérer un fichier, copiez-le depuis ces dossiers. L'onglet **History** de l'app liste ce que chaque synchro a changé.

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
