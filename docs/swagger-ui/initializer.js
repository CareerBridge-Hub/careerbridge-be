window.onload = function () {
  window.ui = SwaggerUIBundle({
    url: '/api/v1/openapi.json',
    dom_id: '#swagger-ui',
    withCredentials: true,
    requestInterceptor: function (request) { request.headers['X-CSRF-Protection'] = '1'; return request; },
    deepLinking: true,
    validatorUrl: null,
    persistAuthorization: false,
    supportedSubmitMethods: ['get', 'post', 'put', 'patch', 'delete'],
    presets: [SwaggerUIBundle.presets.apis],
    layout: 'BaseLayout'
  });
};
