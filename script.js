document.addEventListener('DOMContentLoaded', () => {
    // Copy to clipboard functionality
    const copyBtn = document.getElementById('copyBtn');
    const commandText = document.querySelector('.command').innerText;

    copyBtn.addEventListener('click', () => {
        navigator.clipboard.writeText(commandText).then(() => {
            // Show success state
            const originalHTML = copyBtn.innerHTML;
            copyBtn.innerHTML = '<svg viewBox="0 0 24 24" width="16" height="16" stroke="currentColor" stroke-width="2" fill="none"><polyline points="20 6 9 17 4 12"></polyline></svg>';
            copyBtn.classList.add('copied');

            // Revert after 2 seconds
            setTimeout(() => {
                copyBtn.innerHTML = originalHTML;
                copyBtn.classList.remove('copied');
            }, 2000);
        }).catch(err => {
            console.error('Failed to copy: ', err);
        });
    });

    // Intersection Observer for scroll animations (fade in / slide up)
    const observerOptions = {
        threshold: 0.1,
        rootMargin: "0px 0px -50px 0px"
    };

    const observer = new IntersectionObserver((entries) => {
        entries.forEach(entry => {
            if (entry.isIntersecting) {
                // If it's a feature card, the animation is handled by CSS with delays,
                // but we need to ensure the class is applied when in view if we were adding it dynamically.
                // Currently they have 'slide-up' hardcoded, but this observer allows 
                // re-triggering or adding a 'visible' class if desired for more complex setups.
                entry.target.style.opacity = '1';
                observer.unobserve(entry.target);
            }
        });
    }, observerOptions);

    // Elements with slide-up class
    document.querySelectorAll('.slide-up, .fade-in').forEach(el => {
        // In the CSS we use forward animations so they play immediately.
        // For true scroll-based triggering, we'd add the class here instead.
        // This is a placeholder for future scroll-linked enhancements.
    });
});
