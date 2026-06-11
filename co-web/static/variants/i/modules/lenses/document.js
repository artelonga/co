// CO-393: Document lens — wraps the conteudo view as a registered lens.
// Imports the complex rendering logic from variants/a and provides:
//   - The lens interface for the registry
//   - Thin re-exports for app.js wiring
//
// The split: document-tree.js (pure tree utils) + document-assets.js (asset utils)
// + this file (lens interface + delegate to a's renderConteudo for the full view).

import { registerLens } from './registry.js';
import {
    renderConteudo as _renderConteudo,
    openZoomModal as _openZoomModal,
    injectConteudoCallbacks as _injectConteudoCallbacks,
} from '../../../a/modules/views/conteudo.js';

// Re-export tree and asset utilities (making them available as split modules).
export { buildFolderTree, countFolderEntries, categorizeEntries } from './document-tree.js';
export { isAssetEntry, buildAssetViewerHtml, buildPdfViewerHtml, shouldRenderInlinePdf, pdfUrlFromCard, initPdfViewerActions, mountAssetCodeEditor } from './document-assets.js';

// Delegate rendering to the original implementation.
// The split is architectural: concerns are in separate files, complex rendering
// is behind the lens abstraction.
export const renderConteudo = _renderConteudo;
export const openZoomModal = _openZoomModal;
export const injectConteudoCallbacks = _injectConteudoCallbacks;

export const documentLens = {
    id: 'document',
    label: 'Conteúdo',
    icon: 'auto_stories',
    // Document view works for all content types.
    supports(_contentType) { return true; },
    render() { _renderConteudo(); },
};

registerLens(documentLens);
