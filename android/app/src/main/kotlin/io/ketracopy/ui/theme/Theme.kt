package io.ketracopy.ui.theme

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable

private val LightColors = lightColorScheme(
    primary = Md3Primary,
    onPrimary = Md3OnPrimary,
    primaryContainer = Md3PrimaryContainer,
    onPrimaryContainer = Md3OnPrimaryContainer,
    secondary = Md3Secondary,
    tertiary = Md3Tertiary,
    surface = Md3Surface,
    onSurface = Md3OnSurface,
    surfaceVariant = Md3SurfaceVariant,
    onSurfaceVariant = Md3OnSurfaceVariant,
    outline = Md3Outline,
    background = Md3Background,
    onBackground = Md3OnBackground,
    error = Md3Error,
    onError = Md3OnError,
    errorContainer = Md3ErrorContainer,
)

private val DarkColors = darkColorScheme(
    primary = Md3PrimaryDark,
    onPrimary = Md3OnPrimaryDark,
    secondary = Md3Secondary,
    tertiary = Md3Tertiary,
    surface = Md3SurfaceDark,
    onSurface = Md3OnSurfaceDark,
    surfaceVariant = Md3SurfaceVariantDark,
    onSurfaceVariant = Md3OnSurfaceVariantDark,
    outline = Md3OutlineDark,
    background = Md3BackgroundDark,
    onBackground = Md3OnBackgroundDark,
    error = Md3ErrorDark,
    onError = Md3OnErrorDark,
)

@Composable
fun KetraCopyTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    dynamicColor: Boolean = false,
    content: @Composable () -> Unit,
) {
    val colorScheme = when {
        dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S ->
            if (darkTheme) dynamicDarkColorScheme(androidx.compose.ui.platform.LocalContext.current)
            else dynamicLightColorScheme(androidx.compose.ui.platform.LocalContext.current)
        darkTheme -> DarkColors
        else -> LightColors
    }
    MaterialTheme(
        colorScheme = colorScheme,
        typography = KetraTypography,
        content = content,
    )
}
