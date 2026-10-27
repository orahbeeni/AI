package com.omar.smsmanager.sms

import android.app.role.RoleManager
import android.content.Context
import android.content.Intent
import android.provider.Telephony

object DefaultApp {
    fun isDefault(ctx: Context): Boolean {
        val rm = ctx.getSystemService(RoleManager::class.java)
        if (rm != null && rm.isRoleAvailable(RoleManager.ROLE_SMS)) return rm.isRoleHeld(RoleManager.ROLE_SMS)
        return Telephony.Sms.getDefaultSmsPackage(ctx) == ctx.packageName
    }

    /** The system dialog that asks the user to make us the default SMS app, or null if it isn't available. */
    fun requestIntent(ctx: Context): Intent? {
        val rm = ctx.getSystemService(RoleManager::class.java) ?: return null
        if (!rm.isRoleAvailable(RoleManager.ROLE_SMS)) return null
        return rm.createRequestRoleIntent(RoleManager.ROLE_SMS)
    }
}
