# -*- coding: utf-8 -*-
"""Generate apps/desktop/runtime-assets/i18n/locales/handoff.json for all locales."""
import json, io, collections

KEYS = [
    "handoff.section.title", "handoff.section.empty", "handoff.section.empty.detail",
    "handoff.list.revision_format", "handoff.list.artifacts_format", "handoff.list.next_steps_format",
    "handoff.status.inProgress", "handoff.status.completed", "handoff.status.interrupted",
    "handoff.status.blocked", "handoff.status.stale",
    "handoff.artifact.kind.plan", "handoff.artifact.kind.task", "handoff.artifact.kind.result",
    "handoff.artifact.kind.diff", "handoff.artifact.kind.changedFiles",
    "handoff.artifact.kind.transcript", "handoff.artifact.kind.file",
    "handoff.artifact.unavailable", "handoff.artifact.drag_hint",
    "handoff.action.open_document", "handoff.action.inject", "handoff.action.force",
    "handoff.action.rename", "handoff.action.delete", "handoff.action.copy_path",
    "handoff.action.delete.title", "handoff.action.delete.confirm_format", "handoff.action.rename.title",
    "handoff.message.built_format", "handoff.message.reused_format", "handoff.message.build_failed_format",
    "handoff.message.delivered_format", "handoff.message.delivery_failed_format",
    "handoff.message.unavailable", "handoff.message.not_found", "handoff.message.deleted",
    "handoff.detail.title", "handoff.detail.goal", "handoff.detail.unfinished", "handoff.detail.blockers",
    "handoff.detail.facts", "handoff.detail.constraints", "handoff.detail.decisions",
    "handoff.detail.open_items", "handoff.detail.artifacts", "handoff.detail.final_result",
    "handoff.detail.advisory_notice", "handoff.detail.environment", "handoff.detail.empty",
]

T = {}

T["en"] = [
    "Handoffs", "No handoffs yet", "A record appears here after each AI turn finishes.",
    "rev %@", "%@ artifacts", "%@ unfinished", "In progress", "Completed", "Interrupted", "Blocked", "Stale",
    "Plan", "Tasks", "Result", "Diff", "Changed files", "Transcript", "File",
    "Source changed - not delivered", "Drag into a terminal to attach",
    "Open handoff.md", "Inject into Terminal", "Record Handoff Point", "Rename", "Delete", "Copy Artifact Path",
    "Delete Handoff", "Delete the handoff \"%@\" and its staged artifacts?", "Rename Handoff",
    "Recorded handoff for %@.", "Handoff for %@ is already current.", "Failed to record handoff: %@",
    "Attached %@ to the terminal.", "Failed to attach %@: %@",
    "That artifact's source changed and was not attached.", "Handoff record not found.", "Handoff deleted.",
    "Handoff Detail", "Goal", "Unfinished", "Blockers", "Facts", "Constraints", "Decisions", "Open items",
    "Artifacts", "Final result (advisory)", "Model output captured at handoff time; it may be stale.",
    "Environment", "Nothing was captured for this handoff.",
]

T["zh-Hans"] = [
    "交接记录", "暂无交接记录", "每轮 AI 对话结束后，这里会出现一条记录。",
    "第 %@ 版", "%@ 个产物", "%@ 项未完成", "进行中", "已完成", "已中断", "受阻", "已过期",
    "计划", "任务", "结果", "差异", "变更文件", "会话记录", "文件",
    "源文件已变更 — 未投递", "拖入终端即可作为附件",
    "打开 handoff.md", "注入到终端", "记录交接点", "重命名", "删除", "复制产物路径",
    "删除交接记录", "删除交接记录“%@”及其暂存产物？", "重命名交接记录",
    "已记录 %@ 的交接。", "%@ 的交接记录已是最新。", "记录交接失败：%@",
    "已将 %@ 附加到终端。", "附加 %@ 失败：%@",
    "该产物的源文件已变更，未附加。", "未找到交接记录。", "交接记录已删除。",
    "交接详情", "目标", "未完成", "阻塞项", "事实", "约束", "决策", "待确认项",
    "产物", "最终结果（仅供参考）", "模型在交接时刻的输出，可能已过时。",
    "环境", "本次交接没有捕获到内容。",
]

T["zh-Hant"] = [
    "交接記錄", "暫無交接記錄", "每輪 AI 對話結束後，這裡會出現一筆記錄。",
    "第 %@ 版", "%@ 個產物", "%@ 項未完成", "進行中", "已完成", "已中斷", "受阻", "已過期",
    "計畫", "任務", "結果", "差異", "變更檔案", "對話記錄", "檔案",
    "來源檔案已變更 — 未投遞", "拖入終端機即可作為附件",
    "開啟 handoff.md", "注入至終端機", "記錄交接點", "重新命名", "刪除", "複製產物路徑",
    "刪除交接記錄", "刪除交接記錄「%@」及其暫存產物？", "重新命名交接記錄",
    "已記錄 %@ 的交接。", "%@ 的交接記錄已是最新。", "記錄交接失敗：%@",
    "已將 %@ 附加至終端機。", "附加 %@ 失敗：%@",
    "該產物的來源檔案已變更，未附加。", "找不到交接記錄。", "交接記錄已刪除。",
    "交接詳情", "目標", "未完成", "阻塞項", "事實", "約束", "決策", "待確認項",
    "產物", "最終結果（僅供參考）", "模型在交接時刻的輸出，可能已過時。",
    "環境", "本次交接未擷取到內容。",
]

T["ja"] = [
    "引き継ぎ記録", "引き継ぎ記録はまだありません", "AI のターンが完了するたびに、ここに記録が追加されます。",
    "版 %@", "成果物 %@ 件", "未完了 %@ 件", "進行中", "完了", "中断", "ブロック", "古い",
    "プラン", "タスク", "結果", "差分", "変更ファイル", "会話記録", "ファイル",
    "元ファイルが変更されました — 未配送", "ターミナルにドラッグして添付",
    "handoff.md を開く", "ターミナルに挿入", "引き継ぎポイントを記録", "名前を変更", "削除", "成果物パスをコピー",
    "引き継ぎ記録を削除", "引き継ぎ記録「%@」と暫定成果物を削除しますか？", "引き継ぎ記録の名前を変更",
    "%@ の引き継ぎを記録しました。", "%@ の引き継ぎ記録は最新です。", "引き継ぎの記録に失敗しました：%@",
    "%@ をターミナルに添付しました。", "%@ の添付に失敗しました：%@",
    "この成果物の元ファイルが変更されたため、添付しませんでした。", "引き継ぎ記録が見つかりません。", "引き継ぎ記録を削除しました。",
    "引き継ぎ詳細", "目標", "未完了", "ブロッカー", "事実", "制約", "決定", "未確認項目",
    "成果物", "最終結果（参考情報）", "引き継ぎ時点のモデル出力です。古くなっている可能性があります。",
    "環境", "この引き継ぎには何も記録されませんでした。",
]

T["ko"] = [
    "인수인계 기록", "아직 인수인계 기록이 없습니다", "AI 턴이 끝날 때마다 여기에 기록이 생깁니다.",
    "개정 %@", "산출물 %@개", "미완료 %@건", "진행 중", "완료됨", "중단됨", "차단됨", "오래됨",
    "계획", "작업", "결과", "차이", "변경 파일", "대화 기록", "파일",
    "원본 파일이 변경됨 — 전달되지 않음", "터미널로 끌어다 놓으면 첨부됩니다",
    "handoff.md 열기", "터미널에 주입", "인수인계 시점 기록", "이름 바꾸기", "삭제", "산출물 경로 복사",
    "인수인계 기록 삭제", "인수인계 기록 \"%@\" 및 임시 산출물을 삭제할까요?", "인수인계 기록 이름 바꾸기",
    "%@의 인수인계를 기록했습니다.", "%@의 인수인계 기록은 이미 최신입니다.", "인수인계 기록 실패: %@",
    "%@을(를) 터미널에 첨부했습니다.", "%@ 첨부 실패: %@",
    "해당 산출물의 원본이 변경되어 첨부하지 않았습니다.", "인수인계 기록을 찾을 수 없습니다.", "인수인계 기록이 삭제되었습니다.",
    "인수인계 상세", "목표", "미완료", "차단 항목", "사실", "제약", "결정", "미확인 항목",
    "산출물", "최종 결과(참고용)", "인수인계 시점의 모델 출력이며 오래되었을 수 있습니다.",
    "환경", "이 인수인계에 기록된 내용이 없습니다.",
]

T["de"] = [
    "Übergaben", "Noch keine Übergaben", "Nach jedem abgeschlossenen KI-Durchgang erscheint hier ein Eintrag.",
    "Rev. %@", "%@ Artefakte", "%@ offen", "In Arbeit", "Abgeschlossen", "Unterbrochen", "Blockiert", "Veraltet",
    "Plan", "Aufgaben", "Ergebnis", "Diff", "Geänderte Dateien", "Protokoll", "Datei",
    "Quelle geändert - nicht übergeben", "In ein Terminal ziehen, um anzuhängen",
    "handoff.md öffnen", "In Terminal einfügen", "Übergabepunkt festhalten", "Umbenennen", "Löschen", "Artefaktpfad kopieren",
    "Übergabe löschen", "Übergabe \"%@\" und zwischengespeicherte Artefakte löschen?", "Übergabe umbenennen",
    "Übergabe für %@ festgehalten.", "Übergabe für %@ ist bereits aktuell.", "Übergabe konnte nicht festgehalten werden: %@",
    "%@ an das Terminal übergeben.", "%@ konnte nicht angehängt werden: %@",
    "Die Quelle dieses Artefakts hat sich geändert; es wurde nicht angehängt.", "Übergabe nicht gefunden.", "Übergabe gelöscht.",
    "Übergabedetails", "Ziel", "Offen", "Blocker", "Fakten", "Randbedingungen", "Entscheidungen", "Offene Punkte",
    "Artefakte", "Endergebnis (Hinweis)", "Modellausgabe zum Zeitpunkt der Übergabe; möglicherweise veraltet.",
    "Umgebung", "Für diese Übergabe wurde nichts erfasst.",
]

T["fr"] = [
    "Passations", "Aucune passation pour l'instant", "Un enregistrement apparaît ici à la fin de chaque tour d'IA.",
    "rév. %@", "%@ artefacts", "%@ inachevés", "En cours", "Terminé", "Interrompu", "Bloqué", "Obsolète",
    "Plan", "Tâches", "Résultat", "Diff", "Fichiers modifiés", "Transcription", "Fichier",
    "Source modifiée - non transmis", "Glissez dans un terminal pour joindre",
    "Ouvrir handoff.md", "Injecter dans le terminal", "Enregistrer un point de passation", "Renommer", "Supprimer", "Copier le chemin de l'artefact",
    "Supprimer la passation", "Supprimer la passation « %@ » et ses artefacts préparés ?", "Renommer la passation",
    "Passation enregistrée pour %@.", "La passation de %@ est déjà à jour.", "Échec de l'enregistrement de la passation : %@",
    "%@ joint au terminal.", "Échec de la jonction de %@ : %@",
    "La source de cet artefact a changé ; il n'a pas été joint.", "Passation introuvable.", "Passation supprimée.",
    "Détail de la passation", "Objectif", "Inachevé", "Blocages", "Faits", "Contraintes", "Décisions", "Points ouverts",
    "Artefacts", "Résultat final (indicatif)", "Sortie du modèle au moment de la passation ; elle peut être obsolète.",
    "Environnement", "Rien n'a été capturé pour cette passation.",
]

T["es"] = [
    "Traspasos", "Aún no hay traspasos", "Aquí aparece un registro al terminar cada turno de IA.",
    "rev. %@", "%@ artefactos", "%@ pendientes", "En curso", "Completado", "Interrumpido", "Bloqueado", "Obsoleto",
    "Plan", "Tareas", "Resultado", "Diff", "Archivos cambiados", "Transcripción", "Archivo",
    "Origen modificado - no entregado", "Arrastra a un terminal para adjuntar",
    "Abrir handoff.md", "Inyectar en el terminal", "Registrar punto de traspaso", "Renombrar", "Eliminar", "Copiar ruta del artefacto",
    "Eliminar traspaso", "¿Eliminar el traspaso \"%@\" y sus artefactos almacenados?", "Renombrar traspaso",
    "Traspaso de %@ registrado.", "El traspaso de %@ ya está actualizado.", "No se pudo registrar el traspaso: %@",
    "%@ adjuntado al terminal.", "No se pudo adjuntar %@: %@",
    "El origen de ese artefacto cambió y no se adjuntó.", "No se encontró el registro de traspaso.", "Traspaso eliminado.",
    "Detalle del traspaso", "Objetivo", "Pendiente", "Bloqueos", "Hechos", "Restricciones", "Decisiones", "Puntos abiertos",
    "Artefactos", "Resultado final (orientativo)", "Salida del modelo al momento del traspaso; puede estar obsoleta.",
    "Entorno", "No se capturó nada para este traspaso.",
]

T["pt-BR"] = [
    "Handoffs", "Ainda não há handoffs", "Um registro aparece aqui após cada turno de IA concluído.",
    "rev. %@", "%@ artefatos", "%@ pendentes", "Em andamento", "Concluído", "Interrompido", "Bloqueado", "Desatualizado",
    "Plano", "Tarefas", "Resultado", "Diff", "Arquivos alterados", "Transcrição", "Arquivo",
    "Origem alterada - não entregue", "Arraste para um terminal para anexar",
    "Abrir handoff.md", "Injetar no terminal", "Registrar ponto de handoff", "Renomear", "Excluir", "Copiar caminho do artefato",
    "Excluir handoff", "Excluir o handoff \"%@\" e seus artefatos armazenados?", "Renomear handoff",
    "Handoff de %@ registrado.", "O handoff de %@ já está atualizado.", "Falha ao registrar o handoff: %@",
    "%@ anexado ao terminal.", "Falha ao anexar %@: %@",
    "A origem desse artefato mudou e ele não foi anexado.", "Registro de handoff não encontrado.", "Handoff excluído.",
    "Detalhe do handoff", "Objetivo", "Pendente", "Bloqueios", "Fatos", "Restrições", "Decisões", "Itens em aberto",
    "Artefatos", "Resultado final (informativo)", "Saída do modelo no momento do handoff; pode estar desatualizada.",
    "Ambiente", "Nada foi capturado para este handoff.",
]

T["ru"] = [
    "Передачи", "Пока нет передач", "Запись появляется здесь после завершения каждого хода ИИ.",
    "рев. %@", "артефактов: %@", "не завершено: %@", "В работе", "Завершено", "Прервано", "Заблокировано", "Устарело",
    "План", "Задачи", "Результат", "Diff", "Изменённые файлы", "Транскрипт", "Файл",
    "Источник изменён - не доставлено", "Перетащите в терминал, чтобы приложить",
    "Открыть handoff.md", "Вставить в терминал", "Зафиксировать точку передачи", "Переименовать", "Удалить", "Копировать путь артефакта",
    "Удалить передачу", "Удалить передачу \"%@\" и её сохранённые артефакты?", "Переименовать передачу",
    "Передача для %@ зафиксирована.", "Передача для %@ уже актуальна.", "Не удалось зафиксировать передачу: %@",
    "%@ приложено к терминалу.", "Не удалось приложить %@: %@",
    "Источник этого артефакта изменился, он не был приложен.", "Запись передачи не найдена.", "Передача удалена.",
    "Детали передачи", "Цель", "Не завершено", "Блокеры", "Факты", "Ограничения", "Решения", "Открытые вопросы",
    "Артефакты", "Итоговый результат (справочно)", "Вывод модели на момент передачи; может быть устаревшим.",
    "Окружение", "Для этой передачи ничего не зафиксировано.",
]

LANGS = ["de", "en", "es", "fr", "ja", "ko", "pt-BR", "ru", "zh-Hans", "zh-Hant"]

strings = collections.OrderedDict()
for lang in LANGS:
    values = T[lang]
    assert len(values) == len(KEYS), (lang, len(values), len(KEYS))
    strings[lang] = collections.OrderedDict(zip(KEYS, values))

out = collections.OrderedDict([("strings", strings)])
path = "apps/desktop/runtime-assets/i18n/locales/handoff.json"
with io.open(path, "w", encoding="utf-8", newline="\n") as handle:
    json.dump(out, handle, ensure_ascii=False, indent=2)
    handle.write("\n")
print("wrote", len(KEYS), "keys x", len(LANGS), "langs ->", path)
