// i18n.js — CO Web UI translations (shared across all variants)
// Language stored in co_lang cookie; default: pt, detected from navigator.language.

(function () {
    'use strict';

    var I18N = {
        pt: {
            // Spec-defined dot-notation keys
            'nav.projects': 'Projetos',
            'nav.dashboard': 'Painel',
            'action.create': 'Criar',
            'action.edit': 'Editar',
            'action.delete': 'Excluir',
            'action.save': 'Salvar',
            'action.cancel': 'Cancelar',
            'action.clone': 'Criar universo',
            'action.login': 'Entrar',
            'action.logout': 'Sair',
            'action.register': 'Criar conta',
            'action.archive': 'Arquivar',
            'action.comment': 'Comentar',
            'status.todo': 'A fazer',
            'status.in_progress': 'Em progresso',
            'status.in_review': 'Em revisão',
            'status.done': 'Concluído',
            'priority.low': 'Baixa',
            'priority.medium': 'Média',
            'priority.high': 'Alta',
            'priority.critical': 'Crítica',
            'universe.entries': 'entradas',
            'universe.limit': 'Limite atingido',
            'universe.limit_msg': 'Você atingiu {n} entradas. Crie uma conta gratuita para continuar.',
            'universe.settings': 'Configurações do universo',
            'universe.template_title': 'CO — Gestão de conteúdo em grafo',
            'universe.template_sub': 'Quadro demonstrativo — leitura apenas. Clone para começar o seu.',
            'universe.template_tagline': 'Gestão de conteúdo em grafo — livre e aberto',
            'universe.readonly_tooltip': 'Crie seu universo para editar',
            'theme.switch': 'Tema',
            'lang.switch': 'English',
            'form.name': 'Nome',
            'form.slug': 'Slug (URL)',
            'form.slug_hint': 'Apenas letras minúsculas, números e hífens (2–40 caracteres)',
            'form.theme': 'Tema',
            'form.layout': 'Layout padrão',
            'form.font_headline': 'Fonte de títulos (Google Fonts)',
            'form.font_body': 'Fonte de texto (Google Fonts)',
            'form.comments_title': 'Comentários',
            'form.comment_name': 'Nome (opcional)',
            'form.comment_placeholder': 'Adicionar um comentário...',
            'form.move_to': 'Mover para...',
            // Flat keys (backward-compat with existing data-i18n attributes)
            projects: 'Projetos',
            select_project: 'Selecione um projeto',
            select_project_hint: 'Selecione um projeto na barra lateral',
            sign_out: 'Sair',
            kanban: 'Kanban',
            table: 'Tabela',
            timeline: 'Linha do Tempo',
            calendar: 'Calendário',
            dashboard: 'Painel',
            conteudo: 'Conteúdo',
            week: 'Semana',
            month: 'Mês',
            quarter: 'Trimestre',
            today: 'Hoje',
            archived: 'Arquivados',
            filter_tasks: 'Filtrar tarefas... (/)',
            new_task: '+ Nova Tarefa',
            loading: 'Carregando...',
            activity: 'Atividade',
            new_task_title: 'Nova Tarefa',
            title: 'Título',
            status: 'Status',
            todo: 'A Fazer',
            in_progress: 'Em Andamento',
            in_review: 'Em Revisão',
            done: 'Concluído',
            priority: 'Prioridade',
            low: 'Baixa',
            medium: 'Média',
            high: 'Alta',
            critical: 'Crítica',
            due_date: 'Data de Entrega',
            parent_task: 'Tarefa Pai',
            none: 'Nenhuma',
            assignee: 'Responsável',
            name_or_email: 'Nome ou email',
            labels: 'Etiquetas (separadas por vírgula)',
            description: 'Descrição',
            delete: 'Excluir',
            archive: 'Arquivar',
            cancel: 'Cancelar',
            save: 'Salvar',
            'action.github': 'GitHub',
            'footer.tagline': 'CO — código aberto',
            login_title: 'Entrar',
            login_subtitle: 'Acesse seu quadro de projetos',
            username: 'Usuário',
            username_placeholder: 'seu.usuario',
            password: 'Senha',
            sign_in: 'Entrar',
            signing_in: 'Entrando…',
            invalid_credentials: 'Usuário ou senha incorretos.',
            login_error: 'Não foi possível conectar. Tente novamente.',
            settings: 'Configurações do universo',
            board_layout: 'Quadro (Kanban)',
        },
        en: {
            // Spec-defined dot-notation keys
            'nav.projects': 'Projects',
            'nav.dashboard': 'Dashboard',
            'action.create': 'Create',
            'action.edit': 'Edit',
            'action.delete': 'Delete',
            'action.save': 'Save',
            'action.cancel': 'Cancel',
            'action.clone': 'Create universe',
            'action.login': 'Sign In',
            'action.logout': 'Sign Out',
            'action.register': 'Create account',
            'action.archive': 'Archive',
            'action.comment': 'Comment',
            'status.todo': 'To Do',
            'status.in_progress': 'In Progress',
            'status.in_review': 'In Review',
            'status.done': 'Done',
            'priority.low': 'Low',
            'priority.medium': 'Medium',
            'priority.high': 'High',
            'priority.critical': 'Critical',
            'universe.entries': 'entries',
            'universe.limit': 'Limit reached',
            'universe.limit_msg': 'You\'ve reached {n} entries. Create a free account to continue.',
            'universe.settings': 'Universe settings',
            'universe.template_title': 'CO — Graph-based content management',
            'universe.template_sub': 'Demo board — read only. Clone to start your own.',
            'universe.template_tagline': 'Graph-based content management — free and open',
            'universe.readonly_tooltip': 'Create your universe to edit',
            'theme.switch': 'Theme',
            'lang.switch': 'Português',
            'form.name': 'Name',
            'form.slug': 'Slug (URL)',
            'form.slug_hint': 'Lowercase letters, numbers and hyphens (2–40 characters)',
            'form.theme': 'Theme',
            'form.layout': 'Default layout',
            'form.font_headline': 'Headline font (Google Fonts)',
            'form.font_body': 'Body font (Google Fonts)',
            'form.comments_title': 'Comments',
            'form.comment_name': 'Name (optional)',
            'form.comment_placeholder': 'Add a comment...',
            'form.move_to': 'Move to...',
            // Flat keys
            projects: 'Projects',
            select_project: 'Select a project',
            select_project_hint: 'Select a project from the sidebar',
            sign_out: 'Sign out',
            kanban: 'Kanban',
            table: 'Table',
            timeline: 'Timeline',
            calendar: 'Calendar',
            dashboard: 'Dashboard',
            conteudo: 'Content',
            week: 'Week',
            month: 'Month',
            quarter: 'Quarter',
            today: 'Today',
            archived: 'Archived',
            filter_tasks: 'Filter tasks... (/)',
            new_task: '+ New Task',
            loading: 'Loading...',
            activity: 'Activity',
            new_task_title: 'New Task',
            title: 'Title',
            status: 'Status',
            todo: 'To Do',
            in_progress: 'In Progress',
            in_review: 'In Review',
            done: 'Done',
            priority: 'Priority',
            low: 'Low',
            medium: 'Medium',
            high: 'High',
            critical: 'Critical',
            due_date: 'Due Date',
            parent_task: 'Parent Task',
            none: 'None',
            assignee: 'Assignee',
            name_or_email: 'Name or email',
            labels: 'Labels (comma-separated)',
            description: 'Description',
            delete: 'Delete',
            archive: 'Archive',
            cancel: 'Cancel',
            save: 'Save',
            'action.github': 'GitHub',
            'footer.tagline': 'CO — open source',
            login_title: 'Sign In',
            login_subtitle: 'Access your project board',
            username: 'Username',
            username_placeholder: 'your.username',
            password: 'Password',
            sign_in: 'Sign In',
            signing_in: 'Signing in…',
            invalid_credentials: 'Incorrect username or password.',
            login_error: 'Could not connect. Please try again.',
            settings: 'Universe settings',
            board_layout: 'Board (Kanban)',
        },
    };

    var _lang = 'pt';

    function getCookie(name) {
        var prefix = name + '=';
        var parts = document.cookie.split(';');
        for (var i = 0; i < parts.length; i++) {
            var part = parts[i].trim();
            if (part.indexOf(prefix) === 0) {
                return part.slice(prefix.length);
            }
        }
        return null;
    }

    function setCookie(name, value) {
        var maxAge = 365 * 24 * 60 * 60;
        document.cookie = name + '=' + value + '; Path=/; SameSite=Lax; Max-Age=' + maxAge;
    }

    function t(key) {
        return (I18N[_lang] || I18N.pt)[key] || key;
    }

    function setLang(lang) {
        if (lang !== 'pt' && lang !== 'en') return;
        _lang = lang;
        setCookie('co_lang', lang);
        document.querySelectorAll('[data-i18n]').forEach(function (el) {
            el.textContent = t(el.dataset.i18n);
        });
        document.querySelectorAll('[data-i18n-placeholder]').forEach(function (el) {
            el.placeholder = t(el.dataset.i18nPlaceholder);
        });
        document.documentElement.lang = lang === 'pt' ? 'pt-BR' : 'en';
        document.dispatchEvent(new CustomEvent('co:langchange', { detail: { lang: lang } }));
    }

    // Initialize: cookie overrides navigator.language
    var cookieLang = getCookie('co_lang');
    if (cookieLang === 'pt' || cookieLang === 'en') {
        _lang = cookieLang;
    } else {
        var navLang = (navigator.language || 'pt');
        _lang = navLang.toLowerCase().indexOf('pt') === 0 ? 'pt' : 'en';
        setCookie('co_lang', _lang);
    }
    document.documentElement.lang = _lang === 'pt' ? 'pt-BR' : 'en';

    window.I18N = I18N;
    window.t = t;
    window.setLang = setLang;
    Object.defineProperty(window, 'currentLang', {
        get: function () { return _lang; },
        set: function (v) { _lang = v; },
    });
})();
